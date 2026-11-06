# Software crypto backend

The A-side KeyMint implementation uses RustCrypto and ring through the
`kmr-crypto-ommega` trait implementations. The Android application APIs and
KeyMint operation validation select the algorithm, purpose, padding and digest.

## Symmetric operations

- 3DES supports ECB and CBC with NoPadding or PKCS#7. CBC retains the chaining
  value between updates and finalization. PKCS#7 decryption withholds the final
  ciphertext block until finalization, verifies padding lengths 1 through 8, and
  rejects empty or partial-block ciphertext.
- AES-CTR uses the complete 16-byte initial counter block and a big-endian
  128-bit counter for AES-128, AES-192 and AES-256.
- AES-GCM supports AES-128, AES-192 and AES-256 with a 12-byte nonce and
  authentication tags of 12, 13, 14, 15 or 16 bytes. RustCrypto verifies the
  selected tag in constant time before decrypting; unauthenticated plaintext is
  never returned by this backend. AAD and input are buffered until finalization.
- The internal keystore storage helpers retain their separate ciphertext/tag
  representation and require 16-byte GCM tags.
- AES ECB/CBC PKCS#7 decryption rejects non-block-aligned input and padding
  lengths outside 1 through 16.

## RSA signatures

NONEwithRSA with PKCS#1 v1.5 signature padding applies type-1 encoding to the
original message without hashing or a DigestInfo prefix. The RustCrypto private
operation uses blinding. NoPadding remains raw RSA with a fixed-width result;
inputs must fit the modulus size and represent a value strictly below it.
Digest-based PKCS#1 v1.5 and PSS signatures select the requested digest.

## ECDSA signatures

NIST P-224, P-256, P-384 and P-521 keys sign the digest material the operation
provides. A key restricted to a digest shorter than the curve order (SHA-1 on
P-384 or P-521) and a `Digest::None` prehash of any length are both accepted:
the input is read as a big-endian integer, left-padded with zeros when it is
shorter than the order and reduced to its leftmost order bits when it is longer.
The 521-bit P-521 order is not byte-aligned, so that reduction drops seven bits
instead of a byte; a prehash that fits in 520 bits is used unchanged.
`Digest::None` signs the supplied bytes as the prehash, while every other digest
hashes the accumulated input with the requested digest first. Undigested input
longer than one coordinate is truncated at the input boundary, matching AOSP.
Every signature draws fresh nonce material from the platform CSPRNG, so the same
key and input never repeat a signature; if that randomness is unavailable the
operation fails instead of falling back to a deterministic signature.

## Verification

The crypto unit tests cover independent NIST/OpenSSL known-answer vectors,
chunk boundaries, counter carry, PKCS#7 rejection, native GCM tag authentication,
RSA encoding and the storage helper wire format. Android compilation is gated
with the `aarch64-linux-android` target; host execution is supplementary and does
not certify device integration or hardware-backed security.
