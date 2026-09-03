package android.security.rkp;

import android.security.rkp.RemotelyProvisionedKey;

oneway interface IGetKeyCallback {
    @Backing(type="byte")
    enum ErrorCode {
        ERROR_UNKNOWN = 1,
        ERROR_REQUIRES_SECURITY_PATCH = 2,
        ERROR_PENDING_INTERNET_CONNECTIVITY = 3,
        ERROR_PERMANENT = 5,
    }

    void onSuccess(in RemotelyProvisionedKey key);
    void onCancel();
    void onError(ErrorCode error, String description);
}
