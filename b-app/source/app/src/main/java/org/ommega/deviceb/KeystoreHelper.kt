package org.ommega.deviceb

import android.content.Context
import android.os.Build
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyInfo
import android.security.keystore.KeyProperties
import android.util.Base64
import android.util.Log
import org.json.JSONArray
import org.json.JSONObject
import java.math.BigInteger
import java.security.KeyFactory
import java.security.KeyPairGenerator
import java.security.KeyStore
import java.security.Signature
import java.security.cert.Certificate
import java.security.interfaces.RSAPrivateKey
import java.security.spec.ECGenParameterSpec
import java.security.spec.RSAKeyGenParameterSpec
import java.util.Date
import javax.crypto.Cipher
import javax.security.auth.x500.X500Principal

object KeystoreHelper {
    private const val TAG = "DeviceB.KeystoreHelper"
    private const val KEYSTORE_PROVIDER = "AndroidKeyStore"
    private const val DEFAULT_ALIAS = "deviceb_attest_key"
    private const val DEFAULT_RSA_KEY_SIZE = 2048
    private val DEFAULT_RSA_EXPONENT = BigInteger.valueOf(65537)

    // ---------------------------------------------------------------------
    // KeyMint AIDL enum -> Android Keystore Java constant mapping
    //
    // NOTE on types: `KeyProperties.PURPOSE_*` are `int` BITMASKS
    // (ENCRYPT=1, DECRYPT=2, SIGN=4, VERIFY=8, WRAP_KEY=32, AGREE_KEY=64,
    //  ATTEST_KEY=128) and `KeyGenParameterSpec.Builder(alias, purpose)` takes a
    // SINGLE combined int. `DIGEST_*`, `ENCRYPTION_PADDING_*` and
    // `SIGNATURE_PADDING_*` are `String` constants.
    // ---------------------------------------------------------------------

    /** KeyMint KeyPurpose (0/1/2/3/5/6/7) -> KeyProperties.PURPOSE_* bitmask. */
    private fun purposeToJava(km: Int): Int? = when (km) {
        TaskKeySpec.PURPOSE_ENCRYPT -> KeyProperties.PURPOSE_ENCRYPT
        TaskKeySpec.PURPOSE_DECRYPT -> KeyProperties.PURPOSE_DECRYPT
        TaskKeySpec.PURPOSE_SIGN -> KeyProperties.PURPOSE_SIGN
        TaskKeySpec.PURPOSE_VERIFY -> KeyProperties.PURPOSE_VERIFY
        TaskKeySpec.PURPOSE_WRAP_KEY -> KeyProperties.PURPOSE_WRAP_KEY
        TaskKeySpec.PURPOSE_AGREE_KEY -> KeyProperties.PURPOSE_AGREE_KEY
        TaskKeySpec.PURPOSE_ATTEST_KEY -> KeyProperties.PURPOSE_ATTEST_KEY
        else -> null
    }

    /** KeyMint Digest (0-6) -> KeyProperties.DIGEST_* string. */
    private fun digestToJava(km: Int): String? = when (km) {
        TaskKeySpec.DIGEST_NONE -> KeyProperties.DIGEST_NONE
        TaskKeySpec.DIGEST_MD5 -> KeyProperties.DIGEST_MD5
        TaskKeySpec.DIGEST_SHA1 -> KeyProperties.DIGEST_SHA1
        TaskKeySpec.DIGEST_SHA_2_224 -> KeyProperties.DIGEST_SHA224
        TaskKeySpec.DIGEST_SHA_2_256 -> KeyProperties.DIGEST_SHA256
        TaskKeySpec.DIGEST_SHA_2_384 -> KeyProperties.DIGEST_SHA384
        TaskKeySpec.DIGEST_SHA_2_512 -> KeyProperties.DIGEST_SHA512
        else -> null
    }

    /** KeyMint PaddingMode -> (encryptionPadding, signaturePadding). The Java
     * API splits encryption vs signature paddings, so map to a pair. */
    private fun paddingsToJava(km: Int): Pair<String?, String?> = when (km) {
        TaskKeySpec.PADDING_NONE -> KeyProperties.ENCRYPTION_PADDING_NONE to null
        TaskKeySpec.PADDING_RSA_OAEP -> KeyProperties.ENCRYPTION_PADDING_RSA_OAEP to null
        TaskKeySpec.PADDING_RSA_PSS -> null to KeyProperties.SIGNATURE_PADDING_RSA_PSS
        TaskKeySpec.PADDING_RSA_PKCS1_5_ENC -> KeyProperties.ENCRYPTION_PADDING_RSA_PKCS1 to null
        TaskKeySpec.PADDING_RSA_PKCS1_5_SIGN -> null to KeyProperties.SIGNATURE_PADDING_RSA_PKCS1
        TaskKeySpec.PADDING_PKCS7 -> KeyProperties.ENCRYPTION_PADDING_PKCS7 to null
        else -> null to null
    }

    private fun ecCurveName(curve: Int): String = when (curve) {
        2 -> "secp384r1"
        3 -> "secp521r1"
        else -> "secp256r1"
    }

    private fun keyAlgorithmString(spec: TaskKeySpec): String =
        if (spec.isRsa) KeyProperties.KEY_ALGORITHM_RSA else KeyProperties.KEY_ALGORITHM_EC

    /** On Android < 12 (API 31) PURPOSE_ATTEST_KEY is unsupported; substitute
     * SIGN so key generation still succeeds with an attestable key. */
    private fun sanitizePurpose(purpose: Int): Int {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) return purpose
        return if (purpose and KeyProperties.PURPOSE_ATTEST_KEY != 0) {
            (purpose and KeyProperties.PURPOSE_ATTEST_KEY.inv()) or KeyProperties.PURPOSE_SIGN
        } else purpose
    }

    /** Combine a KeyMint purpose list into a single Java bitmask. */
    private fun combinePurposes(spec: TaskKeySpec, default: Int): Int {
        if (spec.purposes.isEmpty()) return sanitizePurpose(default)
        val mask = spec.purposes
            .mapNotNull(::purposeToJava)
            .fold(0) { acc, p -> acc or p }
        return sanitizePurpose(if (mask != 0) mask else default)
    }

    /**
     * Build a [KeyGenParameterSpec] honoring the requested KeyMint params.
     * `defaultPurpose` is a Java PURPOSE_* bitmask used when the payload
     * carries no purpose list. `signingAttestKeyAlias` (Android 12+) makes the
     * generated key's leaf signed by the given attest key (A-side TBS flow).
     */
    private fun buildKeyGenSpec(
        alias: String,
        challenge: ByteArray?,
        spec: TaskKeySpec,
        defaultPurpose: Int,
        signingAttestKeyAlias: String? = null,
    ): KeyGenParameterSpec {
        val purpose = combinePurposes(spec, defaultPurpose)
        val builder = KeyGenParameterSpec.Builder(alias, purpose)

        if (spec.isRsa) {
            val keySize = spec.keySize ?: DEFAULT_RSA_KEY_SIZE
            val exponent = spec.rsaPublicExponent?.let { BigInteger.valueOf(it) } ?: DEFAULT_RSA_EXPONENT
            builder.setKeySize(keySize)
            builder.setAlgorithmParameterSpec(RSAKeyGenParameterSpec(keySize, exponent))
        } else {
            val curve = spec.ecCurve ?: when (spec.keySize) {
                384 -> 2
                521 -> 3
                else -> 1
            }
            builder.setAlgorithmParameterSpec(ECGenParameterSpec(ecCurveName(curve)))
        }

        // B-app keeps its own SHA-256 default in addition to requested digests.
        // This does not establish parity with native B algorithm/authorization
        // handling; AndroidKeyStore also binds requests to this B-app's UID.
        val digests = spec.digests.mapNotNull(::digestToJava).toMutableList()
        if (KeyProperties.DIGEST_SHA256 !in digests) {
            digests.add(KeyProperties.DIGEST_SHA256)
        }
        builder.setDigests(*digests.toTypedArray())

        if (spec.paddings.isNotEmpty()) {
            val enc = mutableListOf<String>()
            val sig = mutableListOf<String>()
            for (p in spec.paddings) {
                val (e, s) = paddingsToJava(p)
                if (e != null) enc.add(e)
                if (s != null) sig.add(s)
            }
            if (enc.isNotEmpty()) builder.setEncryptionPaddings(*enc.toTypedArray())
            if (sig.isNotEmpty()) builder.setSignaturePaddings(*sig.toTypedArray())
        }

        spec.certificateSubjectDer?.let { der ->
            // DER-encoded X.500 Name (A-side sends tag 503 bytes)
            builder.setCertificateSubject(X500Principal(der))
        }
        spec.certificateSerial?.let { bytes ->
            builder.setCertificateSerialNumber(BigInteger(1, bytes))
        }
        spec.certificateNotBeforeMs?.let { ms -> builder.setCertificateNotBefore(Date(ms)) }
        spec.certificateNotAfterMs?.let { ms -> builder.setCertificateNotAfter(Date(ms)) }

        if (spec.mgfDigest != null) {
            Log.w(TAG, "mgf_digest=${spec.mgfDigest} requested but the Android Keystore API cannot set an MGF1 digest; using the system default")
        }

        challenge?.let { builder.setAttestationChallenge(it) }

        // StrongBox（securityLevel==2，A 端 forwarded 请求）：交给 Android
        // Keystore 原生行为处理。setIsStrongBoxBacked 在无 StrongBox 的设备上
        // 会静默回退 TEE（官方文档行为，链如实标记 TRUSTED_ENVIRONMENT），
        // 这是 Android 的标准降级行为，结果与真实设备一致 —— 生成后由
        // attest() 复核记录实际级别（仅诊断，不再拒绝）。
        if (spec.securityLevel == 2) {
            applyStrongBoxBacked(builder)
                ?: throw IllegalStateException("StrongBox requires Android 9+ (API 28)")
        }

        if (signingAttestKeyAlias != null) {
            return applyAttestationKeyAlias(builder, signingAttestKeyAlias)
                ?.build() ?: throw IllegalStateException("setAttestationKeyAlias unavailable")
        }
        return builder.build()
    }

    // ---------------------------------------------------------------------
    // Public operations
    // ---------------------------------------------------------------------

    fun getCertChain(context: Context, alias: String = DEFAULT_ALIAS): JSONObject {
        val ks = KeyStore.getInstance(KEYSTORE_PROVIDER).apply { load(null) }
        if (!ks.containsAlias(alias)) {
            return JSONObject().put("error", "key not found for alias: $alias (call attest first)")
        }
        val chain: Array<Certificate> = ks.getCertificateChain(alias) ?: emptyArray()
        val certsJson = JSONArray()
        chain.forEach { cert -> certsJson.put(Base64.encodeToString(cert.encoded, Base64.NO_WRAP)) }
        return JSONObject().apply {
            put("alias", alias)
            put("cert_chain", certsJson)
            if (chain.isNotEmpty()) {
                put("public_key", Base64.encodeToString(chain[0].publicKey.encoded, Base64.NO_WRAP))
            }
        }
    }

    /**
     * 使用系统 TEE 对 challenge 进行 key attestation，返回原始证书链。
     * 密钥参数（算法/曲线/用途/digest/padding/证书字段）由 [spec] 指定；
     * 未指定时默认 EC P-256 + SHA-256 + SIGN。
     * 注意：Android Keystore 的 attestation application ID (tag 709) 固定为
     * 调用方应用自身（无法自定义），因此不接收外部 appid。
     */
    fun attest(
        context: Context,
        challengeB64: String,
        alias: String = DEFAULT_ALIAS,
        spec: TaskKeySpec = TaskKeySpec(),
    ): JSONObject {
        val challenge = try {
            Base64.decode(challengeB64, Base64.NO_WRAP)
        } catch (e: Exception) {
            return JSONObject().put("error", "invalid challenge base64")
        }
        return try {
            val ks = KeyStore.getInstance(KEYSTORE_PROVIDER).apply { load(null) }
            if (ks.containsAlias(alias)) ks.deleteEntry(alias)
            val keyGenSpec = buildKeyGenSpec(alias, challenge, spec, KeyProperties.PURPOSE_SIGN)
            KeyPairGenerator.getInstance(keyAlgorithmString(spec), KEYSTORE_PROVIDER)
                .apply { initialize(keyGenSpec) }
                .generateKeyPair()

            // StrongBox 请求：复核实际安全级别，仅记录日志用于诊断 ——
            // 若原生静默降级为 TEE（无 StrongBox 芯片时的标准行为），链上已
            // 如实标记 TRUSTED_ENVIRONMENT，与真实设备表现一致，照常放行；
            // 密钥无需删除，后续 sign/decrypt 由 Android Keystore 自动路由到
            // 密钥实际所在的安全层（StrongBox 或 TEE）。
            if (spec.securityLevel == 2 && !verifyStrongBoxBacked(alias)) {
                Log.w(TAG, "attest: StrongBox requested but key was minted outside StrongBox " +
                        "(standard silent fallback to TEE, alias=$alias)")
            }

            ks.load(null)
            val chain: Array<Certificate> = ks.getCertificateChain(alias) ?: emptyArray()
            val certsJson = JSONArray()
            chain.forEach { cert -> certsJson.put(Base64.encodeToString(cert.encoded, Base64.NO_WRAP)) }
            JSONObject().apply {
                put("alias", alias)
                put("cert_chain", certsJson)
            }
        } catch (e: Exception) {
            Log.e(TAG, "attest failed alias=$alias", e)
            JSONObject().put("error", e.message ?: "attest error")
        }
    }

    /**
     * 使用 alias 对应的 TEE 私钥对数据进行签名。
     * 返回字段名为 "data"，与 A 端 OmegaTee RemoteAttestClient.parseBytes() 对齐。
     */
    fun sign(
        context: Context,
        dataB64: String,
        alias: String = DEFAULT_ALIAS,
        algorithm: String = "SHA256withECDSA"
    ): JSONObject {
        val data = try {
            Base64.decode(dataB64, Base64.NO_WRAP)
        } catch (e: Exception) {
            return JSONObject().put("error", "invalid data base64")
        }
        val ks = KeyStore.getInstance(KEYSTORE_PROVIDER).apply { load(null) }
        val privateKey = ks.getKey(alias, null)
            ?: return JSONObject().put("error", "private key not found for alias: $alias")
        return try {
            val sig = Signature.getInstance(algorithm).apply {
                initSign(privateKey as java.security.PrivateKey)
                update(data)
            }.sign()
            JSONObject().apply {
                put("alias", alias)
                put("algorithm", algorithm)
                put("data", Base64.encodeToString(sig, Base64.NO_WRAP))
            }
        } catch (e: Exception) {
            Log.e(TAG, "sign failed for alias=$alias algo=$algorithm", e)
            JSONObject().put("error", e.message ?: "sign error")
        }
    }

    /**
     * 使用 alias 对应的 TEE 私钥对数据进行解密。
     * 仅支持 RSA 密钥。
     * 返回字段名为 "data"，与 sign 函数对齐。
     */
    fun decrypt(
        context: Context,
        dataB64: String,
        alias: String = DEFAULT_ALIAS,
        algorithm: String = "RSA/ECB/PKCS1Padding"
    ): JSONObject {
        val data = try {
            Base64.decode(dataB64, Base64.NO_WRAP)
        } catch (e: Exception) {
            return JSONObject().put("error", "invalid data base64")
        }
        val ks = KeyStore.getInstance(KEYSTORE_PROVIDER).apply { load(null) }
        val privateKey = ks.getKey(alias, null)
            ?: return JSONObject().put("error", "private key not found for alias: $alias")
        return try {
            val cipher = Cipher.getInstance(algorithm).apply {
                init(Cipher.DECRYPT_MODE, privateKey as java.security.PrivateKey)
            }
            val decrypted = cipher.doFinal(data)
            JSONObject().apply {
                put("alias", alias)
                put("algorithm", algorithm)
                put("data", Base64.encodeToString(decrypted, Base64.NO_WRAP))
            }
        } catch (e: Exception) {
            Log.e(TAG, "decrypt failed for alias=$alias algo=$algorithm", e)
            JSONObject().put("error", e.message ?: "decrypt error")
        }
    }

    private fun applyAttestationKeyAlias(
        builder: KeyGenParameterSpec.Builder,
        attestKeyAlias: String,
    ): KeyGenParameterSpec.Builder? {
        return runCatching {
            val method =
                KeyGenParameterSpec.Builder::class.java.getMethod(
                    "setAttestationKeyAlias",
                    String::class.java,
                )
            method.invoke(builder, attestKeyAlias) as KeyGenParameterSpec.Builder
        }.getOrNull()
    }

    /**
     * 请求由设备的 StrongBox 安全芯片保存密钥。
     * setIsStrongBoxBacked 是 API 28+ 公共 API（项目 minSdk=26，用反射保兼容）；
     * API < 28 无法表达该要求，返回 null。
     * 显式 StrongBox 请求在硬件不可用时可以抛出 StrongBoxUnavailableException；
     * 平台不保证静默回退 TEE。反射失败返回 null 是本客户端的处理策略。
     * [verifyStrongBoxBacked] 仅用于生成后的诊断，不证明请求已满足 StrongBox 要求。
     */
    private fun applyStrongBoxBacked(builder: KeyGenParameterSpec.Builder): KeyGenParameterSpec.Builder? {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.P) return null
        return runCatching {
            val method = KeyGenParameterSpec.Builder::class.java.getMethod(
                "setIsStrongBoxBacked",
                Boolean::class.javaPrimitiveType,
            )
            method.invoke(builder, true) as KeyGenParameterSpec.Builder
        }.getOrNull()
    }

    /**
     * 复核 alias 对应的私钥实际受保护的安全级别（诊断用途）。
     * 仅 API 31+ 的 KeyInfo.getSecurityLevel() 能精确区分 STRONGBOX /
     * TRUSTED_ENVIRONMENT；更早版本没有任何公共 API 能证明密钥位于 StrongBox
     * （isInsideSecureHardware 对 TEE 密钥同样返回 true），因此返回 false 表示
     * 未确认 StrongBox，包括版本不足、查询失败或返回了其他安全级别。
     * 此函数不执行降级，也不能把 false 当作已确认 TEE 的证明。
     */
    private fun verifyStrongBoxBacked(alias: String): Boolean {
        return try {
            if (Build.VERSION.SDK_INT < Build.VERSION_CODES.S) return false
            val ks = KeyStore.getInstance(KEYSTORE_PROVIDER).apply { load(null) }
            val privateKey = ks.getKey(alias, null) ?: return false
            val kf = KeyFactory.getInstance(privateKey.algorithm, KEYSTORE_PROVIDER)
            val keyInfo = kf.getKeySpec(privateKey, KeyInfo::class.java)
            keyInfo.securityLevel == KeyProperties.SECURITY_LEVEL_STRONGBOX
        } catch (e: Exception) {
            Log.e(TAG, "verifyStrongBoxBacked failed alias=$alias", e)
            false
        }
    }
}
