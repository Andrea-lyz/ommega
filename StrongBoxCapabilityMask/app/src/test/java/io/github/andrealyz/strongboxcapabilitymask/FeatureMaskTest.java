package io.github.andrealyz.strongboxcapabilitymask;

import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;

import org.junit.Test;

import java.util.HashMap;
import java.util.Map;

public class FeatureMaskTest {
    private static final String APP_ATTEST_FEATURE =
            "android.hardware.keystore.app_attest_key";

    @Test
    public void removesOnlyStrongBoxFeature() {
        Map<String, Object> features = new HashMap<>();
        features.put(FeatureMask.STRONGBOX_FEATURE, new Object());
        features.put(APP_ATTEST_FEATURE, new Object());

        assertTrue(FeatureMask.removeStrongBoxFeature(features));
        assertFalse(features.containsKey(FeatureMask.STRONGBOX_FEATURE));
        assertTrue(features.containsKey(APP_ATTEST_FEATURE));
    }

    @Test
    public void removalIsIdempotent() {
        Map<String, Object> features = new HashMap<>();

        assertFalse(FeatureMask.removeStrongBoxFeature(features));
        assertFalse(FeatureMask.removeStrongBoxFeature(features));
    }

    @Test
    public void ignoresUnexpectedReturnType() {
        assertFalse(FeatureMask.removeStrongBoxFeature(null));
        assertFalse(FeatureMask.removeStrongBoxFeature("not-a-map"));
    }
}
