use cpal::traits::{DeviceTrait, HostTrait};
pub use rt_core::state::{SYSTEM_DEFAULT_AUDIO_DEVICE, get_default_audio_device_name};

pub fn is_relevant_audio_device(name: &str) -> bool {
    let name_lower = name.to_lowercase();
    if name_lower == "null" {
        return false;
    }
    #[cfg(target_os = "linux")]
    if name_lower.starts_with("surround4")
        || name_lower.starts_with("surround5")
        || name_lower.starts_with("surround7")
    {
        return false;
    }
    if name_lower.starts_with("dmix") || name_lower.starts_with("dsnoop") {
        return false;
    }
    if name_lower.starts_with("adsp") {
        return false;
    }
    if name_lower.starts_with("speex") || name_lower.contains("speexrate") {
        return false;
    }
    if name_lower.contains("upmix") || name_lower.contains("vdownmix") {
        return false;
    }
    if name_lower.contains("samplerate") || name_lower.contains("lavrate") {
        return false;
    }
    if name_lower.contains("sonic studio virtual") {
        return false;
    }
    true
}

pub fn format_cpal_device_name(desc: &cpal::DeviceDescription) -> String {
    #[cfg(target_os = "windows")]
    if let Some(friendly) = desc.extended().next()
        && !friendly.trim().is_empty()
    {
        return friendly.to_string();
    }
    desc.name().to_string()
}

pub fn is_default_audio_device_selection(selected: Option<&str>) -> bool {
    match selected {
        None => true,
        Some(s) => {
            let trimmed = s.trim();
            let lower = trimmed.to_lowercase();
            trimmed.is_empty()
                || trimmed.eq_ignore_ascii_case(SYSTEM_DEFAULT_AUDIO_DEVICE)
                || trimmed.eq_ignore_ascii_case("default")
                || trimmed.eq_ignore_ascii_case("default device")
                || lower.starts_with("default alsa output")
                || lower.starts_with("default alsa input")
        }
    }
}

pub fn is_usable_cpal_device_for_mode(desc: &cpal::DeviceDescription, mic: bool) -> bool {
    if desc.name().eq_ignore_ascii_case("Default Device")
        || desc.name().eq_ignore_ascii_case("default")
    {
        return true;
    }
    if mic {
        #[cfg(not(target_os = "android"))]
        {
            !matches!(desc.direction(), cpal::DeviceDirection::Output)
        }
        #[cfg(target_os = "android")]
        {
            let usable_type = matches!(
                desc.device_type(),
                cpal::DeviceType::Microphone
                    | cpal::DeviceType::Headset
                    | cpal::DeviceType::Headphones
                    | cpal::DeviceType::HearingAid
                    | cpal::DeviceType::Dock
            );
            let usable_iface = matches!(
                desc.interface_type(),
                cpal::InterfaceType::BuiltIn
                    | cpal::InterfaceType::Usb
                    | cpal::InterfaceType::Bluetooth
                    | cpal::InterfaceType::Line
            );
            !matches!(desc.direction(), cpal::DeviceDirection::Output)
                && (usable_type || usable_iface)
        }
    } else {
        !matches!(desc.direction(), cpal::DeviceDirection::Input)
    }
}

pub fn get_available_audio_devices(mic: bool) -> Vec<String> {
    let host = cpal::default_host();
    let devices = if mic {
        host.input_devices()
    } else {
        host.output_devices()
    };

    let mut names = vec![SYSTEM_DEFAULT_AUDIO_DEVICE.to_string()];
    if let Ok(devices) = devices {
        for d in devices {
            if let Ok(desc) = d.description() {
                if !is_usable_cpal_device_for_mode(&desc, mic) {
                    continue;
                }
                let name = format_cpal_device_name(&desc);
                if is_relevant_audio_device(&name)
                    && !is_default_audio_device_selection(Some(&name))
                    && !names.contains(&name)
                {
                    names.push(name);
                }
            }
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_relevant_audio_device_filters_virtual_and_preserves_hardware() {
        for rejected in [
            "null",
            "NULL",
            "dmix:CARD=PCH,DEV=0",
            "dsnoop:CARD=PCH",
            "adsp_virtual",
            "speex",
            "speexrate",
            "upmix_6ch",
            "vdownmix",
            "samplerate",
            "lavrate",
            "Sonic Studio Virtual Mixer",
        ] {
            assert!(
                !is_relevant_audio_device(rejected),
                "Expected virtual/helper device '{rejected}' to be filtered out"
            );
        }

        #[cfg(target_os = "linux")]
        for surround in ["surround40", "surround51", "surround71"] {
            assert!(
                !is_relevant_audio_device(surround),
                "Expected Linux ALSA alias '{surround}' to be filtered out"
            );
        }

        for accepted in [
            "PipeWire Sound Server",
            "HDMI / DisplayPort 2 - Navi 31",
            "USB Audio DAC (UAC2)",
            "Speakers (Realtek Audio)",
        ] {
            assert!(
                is_relevant_audio_device(accepted),
                "Expected hardware/server endpoint '{accepted}' to remain relevant"
            );
        }
    }

    #[test]
    fn test_is_default_audio_device_selection_variants() {
        assert!(is_default_audio_device_selection(None));
        assert!(is_default_audio_device_selection(Some("")));
        assert!(is_default_audio_device_selection(Some("   ")));
        assert!(is_default_audio_device_selection(Some(
            SYSTEM_DEFAULT_AUDIO_DEVICE
        )));
        assert!(is_default_audio_device_selection(Some("DEFAULT")));
        assert!(is_default_audio_device_selection(Some("Default Device")));
        assert!(is_default_audio_device_selection(Some(
            "Default ALSA Output (currently PipeWire Media Server)"
        )));
        assert!(is_default_audio_device_selection(Some(
            "default alsa input (Loopback)"
        )));

        assert!(!is_default_audio_device_selection(Some("HDMI 0")));
        assert!(!is_default_audio_device_selection(Some("RME Babyface Pro")));
    }
}
