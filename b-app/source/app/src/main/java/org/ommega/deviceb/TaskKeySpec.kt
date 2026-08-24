package org.ommega.deviceb

import android.util.Base64
import android.util.Log
import org.json.JSONObject

/**
 * Key-generation parameters requested by the A-side, mirroring the binary
 * B-side's `parse_key_spec`. Values are the raw KeyMint AIDL enum numbers as
 * sent by ommegaclient-a inside `device_attest_context` (top-level fallback is
 * accepted for older relays).
 *
 *   Algorithm:   1 = RSA, 3 = EC
 *   EcCurve:     1 = P-256, 2 = P-384, 3 = P-521
 *   KeyPurpose:  0=ENCRYPT 1=DECRYPT 2=SIGN 3=VERIFY 5=WRAP_KEY 6=AGREE_KEY 7=ATTEST_KEY
 *   Digest:      0=NONE 1=MD5 2=SHA1 3=SHA224 4=SHA256 5=SHA384 6=SHA512
 *   PaddingMode: 1=NONE 2=RSA_OAEP 3=RSA_PSS 4=RSA_PKCS1_5_ENC 5=RSA_PKCS1_5_SIGN 64=PKCS7
 */
data class TaskKeySpec(
    val algorithm: Int = ALGORITHM_EC,
    /** Requested KeyMint security level (raw enum: 1 = TEE, 2 = StrongBox),
     *  forwarded by the relay as `attestation_security_level`. */
    val securityLevel: Int? = null,
    val keySize: Int? = null,
    val ecCurve: Int? = null,
    val purposes: List<Int> = emptyList(),
    val digests: List<Int> = emptyList(),
    val mgfDigest: Int? = null,
    val paddings: List<Int> = emptyList(),
    val rsaPublicExponent: Long? = null,
    val certificateSubjectDer: ByteArray? = null,
    val certificateNotBeforeMs: Long? = null,
    val certificateNotAfterMs: Long? = null,
    val certificateSerial: ByteArray? = null,
) {
    val isRsa: Boolean get() = algorithm == ALGORITHM_RSA

    companion object {
        private const val TAG = "DeviceB.TaskKeySpec"

        const val ALGORITHM_RSA = 1
        const val ALGORITHM_EC = 3

        // KeyMint KeyPurpose (note: no value 4; legacy WRAP_KEY=4 was removed)
        const val PURPOSE_ENCRYPT = 0
        const val PURPOSE_DECRYPT = 1
        const val PURPOSE_SIGN = 2
        const val PURPOSE_VERIFY = 3
        const val PURPOSE_WRAP_KEY = 5
        const val PURPOSE_AGREE_KEY = 6
        const val PURPOSE_ATTEST_KEY = 7

        // KeyMint Digest
        const val DIGEST_NONE = 0
        const val DIGEST_MD5 = 1
        const val DIGEST_SHA1 = 2
        const val DIGEST_SHA_2_224 = 3
        const val DIGEST_SHA_2_256 = 4
        const val DIGEST_SHA_2_384 = 5
        const val DIGEST_SHA_2_512 = 6

        // KeyMint PaddingMode
        const val PADDING_NONE = 1
        const val PADDING_RSA_OAEP = 2
        const val PADDING_RSA_PSS = 3
        const val PADDING_RSA_PKCS1_5_ENC = 4
        const val PADDING_RSA_PKCS1_5_SIGN = 5
        const val PADDING_PKCS7 = 64

        /**
         * Parse a relay task payload. Key-spec fields may live at the top
         * level or nested under `device_attest_context` (top-level wins,
         * mirroring the binary B-side `parse_key_spec`).
         */
        fun fromPayload(payload: JSONObject): TaskKeySpec {
            val nested = payload.optJSONObject("device_attest_context")

            fun has(k: String): Boolean = payload.has(k) || (nested != null && nested.has(k))
            fun optLong(k: String): Long? {
                val v = if (payload.has(k)) payload.opt(k) else nested?.opt(k) ?: return null
                return (v as? Number)?.toLong()
            }
            fun optIntArray(k: String): List<Int> {
                val arr = if (payload.has(k)) payload.optJSONArray(k)
                else nested?.optJSONArray(k) ?: return emptyList()
                return (0 until arr.length()).map { arr.optInt(it) }
            }
            fun optB64(k: String): ByteArray? {
                val s = if (payload.has(k)) payload.optString(k) else nested?.optString(k)
                if (s.isNullOrEmpty()) return null
                return try {
                    Base64.decode(s, Base64.NO_WRAP)
                } catch (e: Exception) {
                    Log.w(TAG, "invalid base64 for $k: ${e.message}")
                    null
                }
            }

            var algorithm = ALGORITHM_EC
            if (has("key_algorithm")) {
                when (optLong("key_algorithm")?.toInt()) {
                    ALGORITHM_RSA -> algorithm = ALGORITHM_RSA
                    ALGORITHM_EC -> algorithm = ALGORITHM_EC
                    else -> Log.w(TAG, "unsupported key_algorithm, defaulting to EC")
                }
            } else if (has("algorithm")) {
                val s = (if (payload.has("algorithm")) payload.optString("algorithm")
                    else nested?.optString("algorithm")).orEmpty().uppercase()
                if (s.contains("RSA")) algorithm = ALGORITHM_RSA
            }

            return TaskKeySpec(
                algorithm = algorithm,
                securityLevel = optLong("attestation_security_level")?.toInt()
                    ?: optLong("security_level")?.toInt(),
                keySize = optLong("key_size")?.toInt(),
                ecCurve = optLong("ec_curve")?.toInt(),
                purposes = optIntArray("purpose"),
                digests = optIntArray("digest"),
                mgfDigest = optLong("mgf_digest")?.toInt(),
                paddings = optIntArray("padding"),
                rsaPublicExponent = optLong("rsa_public_exponent"),
                certificateSubjectDer = optB64("certificate_subject"),
                certificateNotBeforeMs = optLong("certificate_not_before_ms"),
                certificateNotAfterMs = optLong("certificate_not_after_ms"),
                certificateSerial = optB64("certificate_serial"),
            )
        }
    }
}
