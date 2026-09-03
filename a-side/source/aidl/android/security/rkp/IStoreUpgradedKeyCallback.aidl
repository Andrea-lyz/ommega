package android.security.rkp;

oneway interface IStoreUpgradedKeyCallback {
    void onSuccess();
    void onError(String error);
}
