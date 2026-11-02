use std::{collections::HashSet, fmt};

use anyhow::{Context, Result};
use serde::{
    Deserialize,
    de::{DeserializeOwned, DeserializeSeed, Error, MapAccess, SeqAccess, Visitor},
};
use zeroize::Zeroize;

pub(super) fn parse<T: DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    let mut decoder = serde_json::Deserializer::from_slice(bytes);
    JsonDepth(0)
        .deserialize(&mut decoder)
        .context("invalid import JSON structure")?;
    decoder.end().context("trailing import JSON data")?;
    serde_json::from_slice(bytes)
        .map_err(|_| anyhow::anyhow!("invalid import JSON fields or values"))
}

struct JsonDepth(usize);

impl<'de> DeserializeSeed<'de> for JsonDepth {
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(
        self,
        decoder: D,
    ) -> std::result::Result<(), D::Error> {
        if self.0 > 16 {
            return Err(D::Error::custom("JSON nesting exceeds 16 levels"));
        }
        decoder.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for JsonDepth {
    type Value = ();
    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("bounded JSON value")
    }
    fn visit_bool<E: Error>(self, _: bool) -> std::result::Result<(), E> {
        Ok(())
    }
    fn visit_i64<E: Error>(self, _: i64) -> std::result::Result<(), E> {
        Ok(())
    }
    fn visit_u64<E: Error>(self, _: u64) -> std::result::Result<(), E> {
        Ok(())
    }
    fn visit_f64<E: Error>(self, _: f64) -> std::result::Result<(), E> {
        Ok(())
    }
    fn visit_str<E: Error>(self, _: &str) -> std::result::Result<(), E> {
        Ok(())
    }
    fn visit_string<E: Error>(self, mut value: String) -> std::result::Result<(), E> {
        value.zeroize();
        Ok(())
    }
    fn visit_unit<E: Error>(self) -> std::result::Result<(), E> {
        Ok(())
    }
    fn visit_none<E: Error>(self) -> std::result::Result<(), E> {
        Ok(())
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> std::result::Result<(), A::Error> {
        let mut count = 0;
        while sequence.next_element_seed(JsonDepth(self.0 + 1))?.is_some() {
            count += 1;
            if count > 64 {
                return Err(A::Error::custom("JSON array exceeds 64 entries"));
            }
        }
        Ok(())
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> std::result::Result<(), A::Error> {
        let mut seen = HashSet::new();
        while let Some(key) = map.next_key::<String>()? {
            if seen.len() >= 64 {
                return Err(A::Error::custom("JSON map exceeds 64 fields"));
            }
            if !seen.insert(key) {
                return Err(A::Error::custom("duplicate JSON field"));
            }
            map.next_value_seed(JsonDepth(self.0 + 1))?;
        }
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StrictDevice {
    brand: String,
    model: String,
    device: String,
    product: String,
    manufacturer: String,
    fused: u64,
    vb_state: String,
    os_version: String,
    security_level: String,
    bootloader_state: String,
    boot_patch_level: u32,
    system_patch_level: u32,
    vendor_patch_level: u32,
    vbmeta_digest: Option<String>,
    #[serde(default)]
    dice_issuer: Option<String>,
    #[serde(default)]
    dice_subject: Option<String>,
}

impl From<StrictDevice> for crate::profile::DeviceInfo {
    fn from(value: StrictDevice) -> Self {
        Self {
            brand: value.brand,
            model: value.model,
            device: value.device,
            product: value.product,
            manufacturer: value.manufacturer,
            fused: value.fused,
            vb_state: value.vb_state,
            os_version: value.os_version,
            security_level: value.security_level,
            bootloader_state: value.bootloader_state,
            boot_patch_level: value.boot_patch_level,
            system_patch_level: value.system_patch_level,
            vendor_patch_level: value.vendor_patch_level,
            vbmeta_digest: value.vbmeta_digest,
            dice_issuer: value.dice_issuer.unwrap_or_default(),
            dice_subject: value.dice_subject.unwrap_or_default(),
        }
    }
}
