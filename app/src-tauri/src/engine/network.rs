//! Which physical routes this machine can actually put bytes on.
//!
//! Windows answers this with `GetAdaptersAddresses`. Everything below that call
//! is deliberately split out as pure functions: the FFI gives us numbers, and
//! the interesting part is deciding what those numbers mean for a user.

/// One usable IPv4 route on one adapter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Adapter {
    pub if_index: u32,
    /// Windows' own display name, e.g. "Wi-Fi" or "Ethernet".
    pub name: String,
    /// Device description, e.g. "Realtek RTL8188EU Wireless LAN 802.11n USB 2.0 Network Adapter".
    pub description: String,
    pub kind: Kind,
    /// Colon-hex MAC, our stable identity for this adapter across refreshes.
    pub mac: Option<String>,
    pub local_ip: String,
    pub gateway: Option<String>,
    pub ipv4_metric: u32,
    pub link_speed_bps: u64,
    /// A tunnel or software interface: reachable, but not a second physical route.
    pub virtual_: bool,
}

/// How the adapter is attached. Drives the badge text in the Networks view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Wifi,
    Ethernet,
    Usb,
    Cellular,
    Tunnel,
    Other,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Wifi => "wifi",
            Kind::Ethernet => "ethernet",
            Kind::Usb => "usb",
            Kind::Cellular => "cellular",
            Kind::Tunnel => "tunnel",
            Kind::Other => "other",
        }
    }
}

// Interface type numbers from the IANA IF-MIB, as reported in `IfType`.
const IF_TYPE_ETHERNET_CSMACD: u32 = 6;
const IF_TYPE_ISO88025_TOKENRING: u32 = 9;
const IF_TYPE_PPP: u32 = 23;
const IF_TYPE_SOFTWARE_LOOPBACK: u32 = 24;
const IF_TYPE_ATM: u32 = 37;
const IF_TYPE_IEEE80211: u32 = 71;
const IF_TYPE_TUNNEL: u32 = 131;
const IF_TYPE_IEEE1394: u32 = 144;
const IF_TYPE_MOBILE_BROADBAND: u32 = 243;

/// Order matters here. A USB Wi-Fi dongle reports `IF_TYPE_IEEE80211` *and* has
/// "USB" in its description; the radio wins, because that is what the user
/// experiences. Only wired interfaces fall through to the description check.
pub fn classify(if_type: u32, description: &str, virtual_: bool) -> Kind {
    let desc = description.to_ascii_lowercase();
    if virtual_ {
        return Kind::Tunnel;
    }
    match if_type {
        IF_TYPE_IEEE80211 => Kind::Wifi,
        IF_TYPE_MOBILE_BROADBAND => Kind::Cellular,
        IF_TYPE_PPP | IF_TYPE_ISO88025_TOKENRING | IF_TYPE_ATM | IF_TYPE_IEEE1394 => Kind::Other,
        IF_TYPE_ETHERNET_CSMACD => {
            // Remote NDIS is how a phone exposes USB tethering on Windows, and
            // a tethered adapter reports as plain ethernet without saying so
            // anywhere else. Windows spells it "Remote NDIS ...", so match the
            // separate word rather than the joined "rndis".
            if desc.contains("ndis")
                || desc.contains("usb") && desc.contains("ethernet")
                || desc.contains("tether")
                || desc.contains("mobile")
                || desc.contains("cellular")
            {
                Kind::Usb
            } else {
                Kind::Ethernet
            }
        }
        IF_TYPE_TUNNEL => Kind::Tunnel,
        _ => {
            if desc.contains("tailscale") || desc.contains("wireguard") || desc.contains("tap") {
                Kind::Tunnel
            } else {
                Kind::Other
            }
        }
    }
}

/// Whether this interface is a route worth showing at all.
pub fn is_usable(if_type: u32, oper_up: bool, has_routable_ipv4: bool) -> bool {
    if_type != IF_TYPE_SOFTWARE_LOOPBACK && oper_up && has_routable_ipv4
}

/// An address we would happily send download traffic from.
///
/// 169.254/16 means DHCP found nothing, and 127/8 is only useful for tests.
pub fn is_routable_ipv4(octets: [u8; 4]) -> bool {
    !(octets[0] == 169 && octets[1] == 254) && octets[0] != 127 && octets != [0, 0, 0, 0]
}

/// Stable key for one adapter across refreshes.
///
/// The MAC is what survives an unplug/replug. Virtual interfaces often report no
/// hardware address at all, so they fall back to the index, which at least stays
/// put for as long as the tunnel exists.
pub fn identity(adapter: &Adapter) -> String {
    match &adapter.mac {
        Some(mac) => mac.clone(),
        None => format!("ifindex:{}", adapter.if_index),
    }
}

/// The label the deck and the cards show. Windows' own name is usually best, but
/// "Ethernet 2" tells a user nothing about which cable it is.
pub fn display_name(adapter: &Adapter) -> String {
    let name = adapter.name.trim();
    if !name.is_empty() {
        return name.to_string();
    }
    adapter
        .description
        .split_whitespace()
        .next()
        .unwrap_or("Network")
        .to_string()
}

/// Sort bonded routes by preference.
///
/// Physical first, then by Windows' own metric. The order matters: a Tailscale
/// tunnel on this machine reports metric 5 against the Ethernet cable's 35, so
/// sorting on metric alone would present a VPN endpoint as the primary route.
pub fn sort_by_preference(adapters: &mut [Adapter]) {
    adapters.sort_by(|a, b| {
        a.virtual_
            .cmp(&b.virtual_)
            .then_with(|| a.ipv4_metric.cmp(&b.ipv4_metric))
            .then_with(|| a.if_index.cmp(&b.if_index))
    });
}

#[cfg(windows)]
mod win {
    use super::*;
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        GAA_FLAG_INCLUDE_GATEWAYS, GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER,
        GAA_FLAG_SKIP_MULTICAST, GET_ADAPTERS_ADDRESSES_FLAGS, GetAdaptersAddresses,
        IP_ADAPTER_ADDRESSES_LH,
    };
    use windows_sys::Win32::NetworkManagement::Ndis::IfOperStatusUp;
    use windows_sys::Win32::Networking::WinSock::{AF_INET, SOCKADDR, SOCKADDR_IN};

    /// The `family` argument is a plain u32 on this call.
    const FAMILY_IPV4: u32 = AF_INET as u32;
    const ERROR_NO_MORE_ITEMS: u32 = 259;
    const ERROR_BUFFER_OVERFLOW: u32 = 111;

    unsafe fn wide(p: *const u16) -> String {
        if p.is_null() {
            return String::new();
        }
        let mut out = Vec::new();
        for i in 0..1024 {
            let unit = *p.add(i);
            if unit == 0 {
                break;
            }
            out.push(unit);
        }
        String::from_utf16_lossy(&out)
    }

    /// IPv4 dotted quad, or None for anything that is not a routable v4 address.
    unsafe fn ipv4(addr: *const SOCKADDR) -> Option<[u8; 4]> {
        if addr.is_null() || (*addr).sa_family != AF_INET {
            return None;
        }
        let sin = addr as *const SOCKADDR_IN;
        let octets = (*sin).sin_addr.S_un.S_addr.to_ne_bytes();
        if is_routable_ipv4(octets) {
            Some(octets)
        } else {
            None
        }
    }

    fn dotted(octets: [u8; 4]) -> String {
        format!("{}.{}.{}.{}", octets[0], octets[1], octets[2], octets[3])
    }

    /// Every usable adapter, in the order Windows prefers them.
    pub fn enumerate() -> Result<Vec<Adapter>, String> {
        let flags: GET_ADAPTERS_ADDRESSES_FLAGS =
            GAA_FLAG_INCLUDE_GATEWAYS | GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST
                | GAA_FLAG_SKIP_DNS_SERVER;
        let mut size: u32 = 15 * 1024;
        let mut buffer: Vec<u8> = vec![0; size as usize];

        for _ in 0..3 {
            let status = unsafe {
                GetAdaptersAddresses(
                    FAMILY_IPV4,
                    flags,
                    std::ptr::null(),
                    buffer.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH,
                    &mut size,
                )
            };
            if status == 0 || status == ERROR_NO_MORE_ITEMS {
                break;
            }
            if status == ERROR_BUFFER_OVERFLOW {
                buffer = vec![0; size as usize];
                continue;
            }
            return Err(format!("GetAdaptersAddresses failed with {status}"));
        }

        let mut out = Vec::new();
        let mut cursor = buffer.as_ptr() as *const IP_ADAPTER_ADDRESSES_LH;
        while !cursor.is_null() {
            let adapter = unsafe { &*cursor };
            let if_index = unsafe { adapter.Anonymous1.Anonymous.IfIndex };
            let if_type = adapter.IfType;
            let oper_up = adapter.OperStatus == IfOperStatusUp;

            let mut local_ip = None;
            let mut unicast = adapter.FirstUnicastAddress;
            while !unicast.is_null() {
                if let Some(octets) = unsafe { ipv4((*unicast).Address.lpSockaddr) } {
                    local_ip = Some(dotted(octets));
                    break;
                }
                unicast = unsafe { (*unicast).Next };
            }

            if !is_usable(if_type, oper_up, local_ip.is_some()) {
                cursor = adapter.Next;
                continue;
            }

            let mut gateway = None;
            let mut gw = adapter.FirstGatewayAddress;
            while !gw.is_null() {
                if let Some(octets) = unsafe { ipv4((*gw).Address.lpSockaddr) } {
                    gateway = Some(dotted(octets));
                    break;
                }
                gw = unsafe { (*gw).Next };
            }

            let name = unsafe { wide(adapter.FriendlyName) };
            let description = unsafe { wide(adapter.Description) };
            let virtual_ = if_type == IF_TYPE_TUNNEL || looks_virtual(&description);
            let mac = (adapter.PhysicalAddressLength > 0)
                .then(|| {
                    let len = adapter.PhysicalAddressLength.min(6) as usize;
                    let bytes = &adapter.PhysicalAddress[..len];
                    let joined: Vec<String> = bytes.iter().map(|b| format!("{b:02X}")).collect();
                    joined.join(":")
                })
                .filter(|s| s != "00:00:00:00:00:00");

            let kind = classify(if_type, &description, virtual_);

            out.push(Adapter {
                if_index,
                name,
                description,
                kind,
                mac,
                local_ip: local_ip.unwrap_or_default(),
                gateway,
                ipv4_metric: adapter.Ipv4Metric,
                link_speed_bps: adapter.ReceiveLinkSpeed,
                virtual_,
            });

            cursor = adapter.Next;
        }

        sort_by_preference(&mut out);
        Ok(out)
    }

    /// Adapters Windows reports with a real `IfType` but no physical medium.
    fn looks_virtual(description: &str) -> bool {
        let d = description.to_ascii_lowercase();
        d.contains("virtual")
            || d.contains("tailscale")
            || d.contains("wireguard")
            || d.contains("wintun")
            || d.contains("tap adapter")
            || d.contains("loopback")
    }
}

#[cfg(windows)]
pub use win::enumerate;

/// Non-Windows builds have no adapter source. The engine treats this as
/// "single default route", which is exactly what it did before this module.
#[cfg(not(windows))]
pub fn enumerate() -> Result<Vec<Adapter>, String> {
    Ok(Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn adapter(name: &str, desc: &str) -> Adapter {
        Adapter {
            if_index: 1,
            name: name.into(),
            description: desc.into(),
            kind: Kind::Other,
            mac: None,
            local_ip: "192.168.1.2".into(),
            gateway: None,
            ipv4_metric: 10,
            link_speed_bps: 0,
            virtual_: false,
        }
    }

    #[test]
    fn a_usb_wi_fi_dongle_is_wifi_not_usb() {
        let kind = classify(
            IF_TYPE_IEEE80211,
            "Realtek RTL8188EU Wireless LAN 802.11n USB 2.0 Network Adapter",
            false,
        );
        assert_eq!(kind, Kind::Wifi, "the radio must win over the USB word");
    }

    #[test]
    fn phone_tethering_reports_as_usb() {
        assert_eq!(
            classify(IF_TYPE_ETHERNET_CSMACD, "Remote NDIS based Internet Sharing Device", false),
            Kind::Usb
        );
        assert_eq!(classify(IF_TYPE_ETHERNET_CSMACD, "USB Ethernet 5", false), Kind::Usb);
    }

    #[test]
    fn a_real_cable_stays_ethernet() {
        assert_eq!(
            classify(IF_TYPE_ETHERNET_CSMACD, "Realtek PCIe GbE Family Controller", false),
            Kind::Ethernet
        );
    }

    #[test]
    fn tunnels_are_marked_so_they_do_not_look_like_extra_bandwidth() {
        assert_eq!(classify(IF_TYPE_TUNNEL, "WAN Miniport (IP)", false), Kind::Tunnel);
        assert_eq!(
            classify(IF_TYPE_ETHERNET_CSMACD, "Tailscale Tunnel", true),
            Kind::Tunnel,
            "a virtual interface never reports as a physical one"
        );
    }

    #[test]
    fn link_local_and_loopback_are_not_routes() {
        assert!(!is_routable_ipv4([169, 254, 9, 194]), "DHCP found nothing");
        assert!(!is_routable_ipv4([127, 0, 0, 1]));
        assert!(is_routable_ipv4([192, 168, 1, 2]));
        assert!(is_routable_ipv4([100, 103, 204, 24]), "carrier-grade NAT still carries");
    }

    #[test]
    fn unusable_interfaces_are_dropped() {
        assert!(!is_usable(IF_TYPE_ETHERNET_CSMACD, false, true), "cable unplugged");
        assert!(!is_usable(IF_TYPE_ETHERNET_CSMACD, true, false), "no address");
        assert!(!is_usable(IF_TYPE_SOFTWARE_LOOPBACK, true, true), "loopback");
        assert!(is_usable(IF_TYPE_ETHERNET_CSMACD, true, true));
    }

    /// The exact numbers this machine reports: the tunnel wins on metric, so
    /// metric alone would put a VPN endpoint in front of the cable.
    #[test]
    fn a_low_metric_tunnel_never_leads_the_physical_cable() {
        let mut list = vec![
            Adapter {
                if_index: 33,
                ipv4_metric: 5,
                virtual_: true,
                ..adapter("Tailscale", "Tailscale Tunnel")
            },
            Adapter {
                if_index: 12,
                ipv4_metric: 35,
                ..adapter("Ethernet", "Realtek PCIe GbE Family Controller")
            },
        ];
        sort_by_preference(&mut list);
        assert_eq!(list[0].name, "Ethernet");
        assert_eq!(list[1].name, "Tailscale");
    }

    #[test]
    fn two_physical_routes_still_sort_by_metric() {
        let mut list = vec![
            Adapter { if_index: 20, ipv4_metric: 300, ..adapter("USB Ethernet", "USB Ethernet 5") },
            Adapter { if_index: 12, ipv4_metric: 35, ..adapter("Ethernet", "Realtek PCIe") },
        ];
        sort_by_preference(&mut list);
        assert_eq!(list[0].name, "Ethernet");
    }

    #[test]
    fn falls_back_to_the_description_when_windows_has_no_name() {
        assert_eq!(display_name(&adapter("", "Tailscale Tunnel")), "Tailscale");
        assert_eq!(display_name(&adapter("  ", "x")), "x");
    }

    #[test]
    fn kinds_have_stable_wire_names() {
        assert_eq!(Kind::Wifi.as_str(), "wifi");
        assert_eq!(Kind::Usb.as_str(), "usb");
        assert_eq!(Kind::Tunnel.as_str(), "tunnel");
    }

    /// Evidence, not a guess: prints what this machine actually reports.
    #[test]
    #[ignore = "reads the live adapter list"]
    fn dump_this_machine() {
        for a in enumerate().expect("enumerate") {
            println!(
                "if{} {:<12} {:<10} ip={} gw={} metric={} speed={} :: {} / {}",
                a.if_index,
                a.kind.as_str(),
                if a.virtual_ { "virtual" } else { "physical" },
                a.local_ip,
                a.gateway.as_deref().unwrap_or("-"),
                a.ipv4_metric,
                a.link_speed_bps,
                a.name,
                a.description
            );
        }
    }
}
