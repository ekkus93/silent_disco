//! Sends the bounded pre-approval host greeting over the already identified TCP peer.

use silent_disco_core::domain::DeviceId;
use silent_disco_core::protocol::{ControlMessage, Hello};
use silent_disco_core::runtime::SessionAdvertisement;
use silent_disco_core::transport::HostTransportNode;

pub(super) fn send_pending_hello(
    node: &dyn HostTransportNode,
    device_id: &DeviceId,
    advertisement: &SessionAdvertisement,
) -> Option<String> {
    let hello = ControlMessage::Hello(Hello {
        session_id: advertisement.session_id.clone(),
        session_name: advertisement.session_name.clone(),
        host_name: advertisement.host_device_id.as_str().to_owned(),
        approval_required: true,
    });
    node.send_pending_control(device_id, &hello)
        .err()
        .map(|error| error.to_string())
}
