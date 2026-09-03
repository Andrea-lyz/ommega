package android.security.rkp;

parcelable RemotelyProvisionedKey {
    byte[] keyBlob;
    byte[] encodedCertChain;
}
