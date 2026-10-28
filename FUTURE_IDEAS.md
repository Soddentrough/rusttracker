# Future Ideas & Roadmap

## 1. Steam Deck USB Audio Class 2.0 (`UAC2`) Gadget & External Hardware Visualizer Mode

### Overview
The Steam Deck's AMD APU (Van Gogh / Sephiroth) includes a **Synopsys DesignWare USB3 (`dwc3`) Dual-Role Controller**. When switched from host-only (`XHCI`) mode to Dual-Role Device (`DRD`) mode in the BIOS, the Steam Deck's USB-C port can operate as a **USB Device (Gadget)** using the Linux kernel's `libcomposite` and **USB Audio Class 2.0 (`usb_f_uac2`)** drivers.

Combined with SteamOS's **PipeWire** audio graph, this enables the Steam Deck running **RustTracker** to act as a driverless USB audio device in both directions:

---

### Operating Modes

#### Mode A: Steam Deck as a USB Audio Source (`RustTracker on Deck ➔ USB-C ➔ Host PC/Mac/Console`)
* **Workflow:** Play tracker modules (`.MOD`, `.XM`, `.IT`, `.S3M`), lossless audio, or multi-track video on the Steam Deck while viewing 60/90 FPS hardware visualizers on the Deck's screen.
* **Audio Routing:** RustTracker outputs PCM audio to the `UAC2_Gadget` PipeWire sink.
* **Host Behavior:** The connected PC/Mac/console sees the Steam Deck as a standard **USB Digital Audio Input (Line-In / Microphone)** and receives bit-perfect digital audio over the USB-C cable.

#### Mode B: Steam Deck as a USB Sound Card + Dedicated Visualizer Display (`Host PC Audio ➔ USB-C ➔ RustTracker on Deck`)
* **Workflow:** Connect the Steam Deck to a desktop PC or laptop via USB-C and use the Deck as an external 1280×800 hardware-accelerated music visualizer and/or DAC.
* **Audio Routing:** The host PC sees **`Steam Deck USB Audio`** as a USB Speaker/DAC. Audio played on the host PC streams over USB-C into the Steam Deck's `UAC2_Gadget` capture source (and optionally loops through PipeWire to the Steam Deck's internal speakers or 3.5mm headphone jack).
* **RustTracker Behavior:** Running RustTracker in Live Input (`--mic`) mode captures the incoming `UAC2_Gadget` PCM stream for real-time 21-shader GPU visualization.

---

### Hardware & OS Prerequisites

#### 1. Enable USB Dual-Role Device (`DRD`) in Steam Deck BIOS
By default, the Steam Deck BIOS locks the USB-C port to `XHCI` (Host-only).
1. Shut down the Steam Deck completely.
2. Hold **Volume Up (`+`)** and press **Power** until the chime sounds, then open **Setup Utility** (BIOS).
3. Navigate to **Advanced** ➔ **USB Configuration** ➔ **USB Dual Role Device**.
4. Change the setting from **`XHCI`** to **`DRD`** (Dual-Role Device), then **Save and Exit**.
5. Once booted into SteamOS, verify that `/sys/class/udc/` lists the `dwc3` controller (e.g., `dwc3.0.auto`).

#### 2. Linux `configfs` UAC2 Gadget Provisioning Script
Once `DRD` is active, a privileged setup script configures the composite USB Audio Class 2.0 gadget:

```bash
#!/bin/bash
set -euo pipefail

modprobe libcomposite
GADGET_DIR="/sys/kernel/config/usb_gadget/rusttracker_uac2"
mkdir -p "$GADGET_DIR"
cd "$GADGET_DIR"

# Linux Foundation Multifunction Composite Gadget IDs
echo 0x1d6b > idVendor
echo 0x0104 > idProduct
echo 0x0100 > bcdDevice
echo 0x0200 > bcdUSB

mkdir -p strings/0x409
echo "00000001"         > strings/0x409/serialnumber
echo "RustTracker"      > strings/0x409/manufacturer
echo "Steam Deck Audio" > strings/0x409/product

mkdir -p configs/c.1/strings/0x409
echo "UAC2 Audio" > configs/c.1/strings/0x409/configuration
echo 250          > configs/c.1/MaxPower

mkdir -p functions/uac2.usb0
# Capture (Host PC -> Steam Deck): Stereo, 16-bit (2 bytes), 48000 Hz
echo 0x3   > functions/uac2.usb0/c_chmask
echo 48000 > functions/uac2.usb0/c_srate
echo 2     > functions/uac2.usb0/c_ssize

# Playback (Steam Deck -> Host PC): Stereo, 16-bit (2 bytes), 48000 Hz
echo 0x3   > functions/uac2.usb0/p_chmask
echo 48000 > functions/uac2.usb0/p_srate
echo 2     > functions/uac2.usb0/p_ssize

ln -sf functions/uac2.usb0 configs/c.1/

# Bind to the first available USB Device Controller (dwc3)
UDC_DEV="$(ls /sys/class/udc | head -n 1)"
if [ -z "$UDC_DEV" ]; then
    echo "Error: No UDC found in /sys/class/udc. Ensure BIOS 'USB Dual Role Device' is set to DRD." >&2
    exit 1
fi
echo "$UDC_DEV" > UDC
echo "Bound UAC2 USB Audio Gadget to $UDC_DEV"
```

---

### Planned RustTracker Features for Seamless Integration

1. **Automated CLI / Systemd Helper (`--setup-usb-audio`):**
   * Add a CLI helper and optional `systemd` unit that detects `/sys/class/udc/*` and configures/tears down the `rusttracker_uac2` gadget cleanly.
2. **In-App / Gamepad Live Input (`--mic`) Toggle:**
   * Add a **"Live Audio Input"** button directly on the Splash Screen and HUD (accessible via controller / Steam Deck buttons) so users can toggle live USB/Bluetooth/microphone visualization without needing `--mic` launch options.
3. **Automatic PipeWire Source/Sink Selection & Loopback:**
   * Automatically detect the `UAC2_Gadget` ALSA/PipeWire node when present and offer an in-app toggle to either:
     * **Transmit Mode (`Deck ➔ USB`):** Route RustTracker's audio output directly to `UAC2_Gadget` playback.
     * **Visualizer / DAC Mode (`USB ➔ Deck`):** Capture incoming `UAC2_Gadget` audio for visualization, with an optional low-latency PipeWire monitor loopback to the Steam Deck's speakers/headphones.
4. **Zero-BIOS Wireless Alternatives (Bluetooth A2DP & PipeWire Network Audio):**
   * Document and support pairing a PC/phone to the Steam Deck over **Bluetooth (A2DP Sink)** or **PipeWire RTP/TCP** over local Wi-Fi / USB-C Ethernet gadget for users who prefer not to toggle BIOS `DRD` mode.
