package io.github.andrealyz.strongboxcapabilitymask;

import android.app.Activity;
import android.app.Instrumentation;
import android.content.Context;
import android.content.pm.PackageManager;
import android.os.Bundle;

public final class FeatureProbeInstrumentation extends Instrumentation {
    private static final String STRONGBOX_FEATURE =
            "android.hardware.strongbox_keystore";
    private static final String APP_ATTEST_FEATURE =
            "android.hardware.keystore.app_attest_key";

    @Override
    public void onCreate(Bundle arguments) {
        super.onCreate(arguments);
        start();
    }

    @Override
    public void onStart() {
        Context context = getTargetContext();
        PackageManager packageManager = context.getPackageManager();
        boolean strongBox = packageManager.hasSystemFeature(STRONGBOX_FEATURE);
        boolean appAttest = packageManager.hasSystemFeature(APP_ATTEST_FEATURE);

        Bundle results = new Bundle();
        results.putString("strongbox", Boolean.toString(strongBox));
        results.putString("app_attest_key", Boolean.toString(appAttest));
        results.putString("verdict", !strongBox && appAttest ? "PASS" : "FAIL");
        finish(!strongBox && appAttest ? Activity.RESULT_OK : Activity.RESULT_CANCELED, results);
    }
}
