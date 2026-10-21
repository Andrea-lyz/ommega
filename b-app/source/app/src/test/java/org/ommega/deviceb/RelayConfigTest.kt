package org.ommega.deviceb

import org.junit.Assert.*
import org.junit.Test

class RelayConfigTest {
    private val valid = RelayConfig("https://example.test:8443/", "device-b", "test-token")
    @Test fun normalizesOnlyValidConfiguration() {
        assertEquals("https://example.test:8443", normalizeRelayConfig(valid)?.url)
        for (url in listOf("file:///tmp/test", "https://user:password@example.test", "https://example.test?token=secret", "https://example.test#fragment")) {
            assertNull(normalizeRelayConfig(valid.copy(url = url)))
        }
        assertNull(normalizeRelayConfig(valid.copy(token = "")))
        assertNull(normalizeRelayConfig(valid.copy(deviceId = "device b")))
    }
    @Test fun overviewNeverEchoesCredentialsOrQueryParameters() {
        assertEquals("https://example.test:8443/path", publicServerUrl("https://user:secret@example.test:8443/path?token=secret#private"))
    }
}
