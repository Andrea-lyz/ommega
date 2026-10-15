package io.github.andrealyz.strongboxcapabilitymask;

import java.util.ArrayList;
import java.util.List;
import java.util.Map;

final class FeatureMask {
    static final String STRONGBOX_FEATURE = "android.hardware.strongbox_keystore";

    // Ommega-routed callers cannot obtain device ID attestation: their chain is
    // minted by the remote real KeyMint, whose TA rejects every ATTESTATION_ID_* tag,
    // and injecting either the remote device's identifiers or local substitutes into
    // that chain would break the caller's own integrity verdicts. Hiding the
    // declaration keeps applications, probes and the system feature cache on the same
    // "not supported" view instead of letting every identifier request fail.
    static final String DEVICE_ID_ATTESTATION_FEATURE =
            "android.software.device_id_attestation";

    private static final List<String> MASKED_FEATURES = List.of(
            STRONGBOX_FEATURE,
            DEVICE_ID_ATTESTATION_FEATURE);

    private FeatureMask() {
    }

    /**
     * Removes every masked feature from the global feature map and returns the
     * names that were actually present.
     */
    static List<String> removeMaskedFeatures(Object featureMap) {
        if (!(featureMap instanceof Map<?, ?> map)) {
            return List.of();
        }
        List<String> removed = new ArrayList<>(MASKED_FEATURES.size());
        for (String feature : MASKED_FEATURES) {
            if (map.remove(feature) != null) {
                removed.add(feature);
            }
        }
        return removed;
    }
}
