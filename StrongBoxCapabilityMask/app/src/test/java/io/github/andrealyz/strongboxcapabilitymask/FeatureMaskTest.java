package io.github.andrealyz.strongboxcapabilitymask;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;

import org.junit.Test;

import java.util.HashMap;
import java.util.List;
import java.util.Map;

public class FeatureMaskTest {
    private static final String APP_ATTEST_FEATURE =
            "android.hardware.keystore.app_attest_key";
    private static final String HARDWARE_KEYSTORE_FEATURE =
            "android.hardware.hardware_keystore";

    @Test
    public void removesEveryMaskedFeatureAndKeepsOthers() {
        Map<String, Object> features = new HashMap<>();
        features.put(FeatureMask.STRONGBOX_FEATURE, new Object());
        features.put(FeatureMask.DEVICE_ID_ATTESTATION_FEATURE, new Object());
        features.put(APP_ATTEST_FEATURE, new Object());
        features.put(HARDWARE_KEYSTORE_FEATURE, new Object());

        List<String> removed = FeatureMask.removeMaskedFeatures(features);

        assertEquals(2, removed.size());
        assertTrue(removed.contains(FeatureMask.STRONGBOX_FEATURE));
        assertTrue(removed.contains(FeatureMask.DEVICE_ID_ATTESTATION_FEATURE));
        assertFalse(features.containsKey(FeatureMask.STRONGBOX_FEATURE));
        assertFalse(features.containsKey(FeatureMask.DEVICE_ID_ATTESTATION_FEATURE));
        assertTrue(features.containsKey(APP_ATTEST_FEATURE));
        assertTrue(features.containsKey(HARDWARE_KEYSTORE_FEATURE));
    }

    @Test
    public void reportsOnlyFeaturesPresentInTheMap() {
        Map<String, Object> features = new HashMap<>();
        features.put(FeatureMask.DEVICE_ID_ATTESTATION_FEATURE, new Object());

        List<String> removed = FeatureMask.removeMaskedFeatures(features);

        assertEquals(List.of(FeatureMask.DEVICE_ID_ATTESTATION_FEATURE), removed);
    }

    @Test
    public void removalIsIdempotent() {
        Map<String, Object> features = new HashMap<>();

        assertTrue(FeatureMask.removeMaskedFeatures(features).isEmpty());
        assertTrue(FeatureMask.removeMaskedFeatures(features).isEmpty());
    }

    @Test
    public void ignoresUnexpectedReturnType() {
        assertTrue(FeatureMask.removeMaskedFeatures(null).isEmpty());
        assertTrue(FeatureMask.removeMaskedFeatures("not-a-map").isEmpty());
    }
}
