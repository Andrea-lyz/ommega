package io.github.andrealyz.strongboxcapabilitymask;

import android.annotation.SuppressLint;
import android.util.Log;

import java.lang.reflect.Method;
import java.util.concurrent.atomic.AtomicBoolean;

import io.github.libxposed.api.XposedInterface;
import io.github.libxposed.api.XposedModule;
import io.github.libxposed.api.XposedModuleInterface;

public final class StrongBoxCapabilityMaskModule extends XposedModule {
    private static final String TAG = "StrongBoxCapabilityMask";
    private static final String SYSTEM_CONFIG_CLASS = "com.android.server.SystemConfig";
    private static final String GET_AVAILABLE_FEATURES_METHOD = "getAvailableFeatures";

    private final AtomicBoolean hookInstalled = new AtomicBoolean();
    private final AtomicBoolean removalLogged = new AtomicBoolean();
    private volatile boolean loadedInSystemServer;

    @Override
    public void onModuleLoaded(XposedModuleInterface.ModuleLoadedParam param) {
        loadedInSystemServer = param.isSystemServer();
        if (loadedInSystemServer) {
            log(Log.INFO, TAG, "event=module_loaded process=" + param.getProcessName()
                    + " api=" + getApiVersion()
                    + " framework=" + getFrameworkName()
                    + " version=" + getFrameworkVersion());
        }
    }

    @Override
    @SuppressLint("BlockedPrivateApi") // SystemConfig is the intentional system_server hook target.
    public void onSystemServerStarting(XposedModuleInterface.SystemServerStartingParam param) {
        if (!loadedInSystemServer || !hookInstalled.compareAndSet(false, true)) {
            return;
        }

        try {
            Class<?> systemConfigClass = Class.forName(
                    SYSTEM_CONFIG_CLASS,
                    false,
                    param.getClassLoader());
            Method getAvailableFeatures = systemConfigClass.getDeclaredMethod(
                    GET_AVAILABLE_FEATURES_METHOD);
            getAvailableFeatures.setAccessible(true);

            hook(getAvailableFeatures)
                    .setId("mask_strongbox_system_feature")
                    .setPriority(XposedInterface.PRIORITY_HIGHEST)
                    .setExceptionMode(XposedInterface.ExceptionMode.DEFAULT)
                    .intercept(chain -> {
                        Object result = chain.proceed();
                        try {
                            if (FeatureMask.removeStrongBoxFeature(result)
                                    && removalLogged.compareAndSet(false, true)) {
                                log(Log.INFO, TAG,
                                        "event=feature_removed name="
                                                + FeatureMask.STRONGBOX_FEATURE);
                            }
                        } catch (Throwable error) {
                            log(Log.ERROR, TAG, "event=feature_remove_failed", error);
                        }
                        return result;
                    });

            log(Log.INFO, TAG,
                    "event=hook_registered target=" + SYSTEM_CONFIG_CLASS + '#'
                            + GET_AVAILABLE_FEATURES_METHOD);
        } catch (Throwable error) {
            hookInstalled.set(false);
            log(Log.ERROR, TAG, "event=hook_install_failed", error);
        }
    }
}
