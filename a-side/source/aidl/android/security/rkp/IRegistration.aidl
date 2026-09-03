package android.security.rkp;

import android.security.rkp.IGetKeyCallback;
import android.security.rkp.IStoreUpgradedKeyCallback;

oneway interface IRegistration {
    void getKey(int keyId, IGetKeyCallback callback);
    void cancelGetKey(IGetKeyCallback callback);
    void storeUpgradedKeyAsync(
        in byte[] oldKeyBlob,
        in byte[] newKeyBlob,
        IStoreUpgradedKeyCallback callback);
}
