package android.security.rkp;

import android.security.rkp.IRegistration;

oneway interface IGetRegistrationCallback {
    void onSuccess(IRegistration registration);
    void onCancel();
    void onError(String error);
}
