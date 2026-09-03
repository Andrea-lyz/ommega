package android.security.rkp;

import android.security.rkp.IGetRegistrationCallback;

oneway interface IRemoteProvisioning {
    void getRegistration(String irpcName, IGetRegistrationCallback callback);
}
