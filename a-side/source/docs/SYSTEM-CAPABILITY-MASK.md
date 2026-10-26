# System capability mask

The A-side Settings page has a **System capability mask** dialog. Its two switches
independently hide `android.hardware.strongbox_keystore` and
`android.software.device_id_attestation`. Both default to off. Enabling a switch
hides that declaration for the whole system, including apps outside KeyMint's
scope. The switches do not enable hardware that the device does not have.

Save settings stores the policy atomically, verifies readback and displays a
restart notice. Returning without saving discards the draft. Saving does not
restart services or the device. The settings are independent of the CPU view,
authentication policy and KeyMint scope.

The Rust helper stores the policy in
`/data/misc/keystore/ommega/data/system-capability-mask.conf`. At `post-fs-data`,
it prepares `system/etc/permissions/ommega-capability-mask.xml` in the A module.
Android's SystemConfig reads its `unavailable-feature` declarations before
publishing capabilities and process feature caches. An invalid policy produces
an empty mask for that boot and remains available for diagnosis.

This requires the root manager's systemless file mounting support. Current
KernelSU versions use a mounting metamodule for the module's `system` directory.
Late-loading after Android has initialized SystemConfig does not apply changes
until the next normal boot. The dialog checks the visible XML and distinguishes
the saved policy from the mounted policy; an absent mount is reported explicitly.

No LSPosed or Zygisk component is used by this feature. Vendor hard-coded feature
answers may bypass SystemConfig and need firmware-specific validation. The mask
does not stop HAL services, change certificates, migrate keys or block explicit
StrongBox requests.

To restore the device's own declarations, turn both switches off, save and restart
A. Another enabled capability-mask module can still hide the same declarations;
disable the standalone StrongBoxCapabilityMask LSP module when using these
controls. Removing or disabling Ommega also removes its XML on the next boot.
This feature and its restart notice apply only to the A-side module.
