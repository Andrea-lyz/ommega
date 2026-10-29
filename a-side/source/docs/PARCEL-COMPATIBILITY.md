# Android Parcelable compatibility

The injector accepts the Android structured-Parcelable wire format used by
keystore2, KeyMint, authorization and maintenance interfaces. A presence marker
of zero represents null. Every nonzero signed 32-bit marker represents a present
object, including negative values. Serialization emits the canonical marker 1.

The injector's AIDL generation step selects
`parcel::impl_android_deserialize_for_parcelable` for structured Parcelable
deserialization. The generated `read_from_parcel` implementation still validates
the sized body, available bytes, field types and union discriminators. A null
value remains an error for a required object, and a present object with a
truncated body remains invalid.

Equivalent presence markers therefore produce equivalent parsed requests and
follow the same caller-selected backend for reads, updates and deletion. This
also applies to optional Parcelable values, array elements and in-place reads.

Regression tests in `ommega-injector/src/parcel/request/tests/serialization.rs`
cover getKeyEntry, updateSubcomponent, deleteKey, optional attestation keys,
in-place reads, null values and truncated bodies.

The matching platform rule appears in `Parcel.readTypedObject` and the AOSP
Rust Binder `impl_deserialize_for_parcelable` implementation: only zero selects
the null branch.
