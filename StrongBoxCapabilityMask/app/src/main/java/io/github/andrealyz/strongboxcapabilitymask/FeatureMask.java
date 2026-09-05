package io.github.andrealyz.strongboxcapabilitymask;

import java.util.Map;

final class FeatureMask {
    static final String STRONGBOX_FEATURE = "android.hardware.strongbox_keystore";

    private FeatureMask() {
    }

    static boolean removeStrongBoxFeature(Object featureMap) {
        if (!(featureMap instanceof Map<?, ?> map)) {
            return false;
        }
        return map.remove(STRONGBOX_FEATURE) != null;
    }
}
