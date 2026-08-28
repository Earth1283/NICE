use std::collections::HashMap;
use std::net::IpAddr;

use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use nicer_proto::SERVICE_TYPE;
use tokio::sync::mpsc;

use crate::config::sanitize_display;
use crate::error::{Error, Result};
use crate::event::Event;
use crate::identity::Fingerprint;

/// `NICE-1.md` §2 and RFC R13. Nothing learned here implies trust, and nothing here
/// triggers a connection.
pub struct Discovery {
    daemon: ServiceDaemon,
    instance: String,
}

impl Discovery {
    pub fn start(
        device: &str,
        port: u16,
        secure: bool,
        fingerprint: Fingerprint,
        events: mpsc::Sender<Event>,
    ) -> Result<Self> {
        let daemon = ServiceDaemon::new().map_err(|e| Error::Discovery(e.to_string()))?;
        let instance = sanitize_display(device, 40);

        let mut properties = HashMap::new();
        properties.insert("version".to_string(), "1".to_string());
        properties.insert("device".to_string(), instance.clone());
        properties.insert(
            "secure".to_string(),
            if secure { "yes" } else { "no" }.to_string(),
        );
        properties.insert("fp".to_string(), fingerprint.to_string());

        let service = ServiceInfo::new(
            SERVICE_TYPE,
            &instance,
            &format!("{instance}.local."),
            "",
            port,
            properties,
        )
        .map_err(|e| Error::Discovery(e.to_string()))?
        .enable_addr_auto();

        daemon
            .register(service)
            .map_err(|e| Error::Discovery(e.to_string()))?;

        let receiver = daemon
            .browse(SERVICE_TYPE)
            .map_err(|e| Error::Discovery(e.to_string()))?;
        let own_instance = instance.clone();

        tokio::spawn(async move {
            while let Ok(event) = receiver.recv_async().await {
                let announcement = match event {
                    ServiceEvent::ServiceResolved(info) => resolve(&info, &own_instance),
                    ServiceEvent::ServiceRemoved(_, fullname) => Some(Event::PeerLost {
                        instance: instance_of(&fullname),
                    }),
                    _ => None,
                };
                if let Some(announcement) = announcement {
                    if events.send(announcement).await.is_err() {
                        break;
                    }
                }
            }
        });

        Ok(Self { daemon, instance })
    }

    pub fn instance(&self) -> &str {
        &self.instance
    }

    pub fn shutdown(&self) {
        let _ = self.daemon.shutdown();
    }
}

fn instance_of(fullname: &str) -> String {
    sanitize_display(fullname.split('.').next().unwrap_or(fullname), 40)
}

fn resolve(info: &ServiceInfo, own_instance: &str) -> Option<Event> {
    let instance = instance_of(info.get_fullname());
    if instance == own_instance {
        return None;
    }
    let addresses: Vec<IpAddr> = info.get_addresses().iter().copied().collect();
    if addresses.is_empty() {
        return None;
    }
    Some(Event::PeerDiscovered {
        device: info
            .get_property_val_str("device")
            .map(|device| sanitize_display(device, 63))
            .unwrap_or_else(|| instance.clone()),
        instance,
        addresses,
        port: info.get_port(),
        secure: info.get_property_val_str("secure") == Some("yes"),
        fingerprint: info
            .get_property_val_str("fp")
            .and_then(|fp| fp.parse::<Fingerprint>().ok()),
    })
}
