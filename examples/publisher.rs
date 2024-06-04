use log::error;
use std::thread;
use std::time::Duration;
use up_client_vsomeip_rust::UPClientVsomeip;
use up_rust::{UMessageBuilder, UTransport, UUri};

#[tokio::main]
async fn main() {
    env_logger::init();

    let app_name = "publisher";
    let authority_name = "foo";
    let ue_id = 10;
    let ue_version_major = 1;
    let resource_id = 0x8001;

    let client_res = UPClientVsomeip::new(authority_name, ue_id);

    let Ok(client) = client_res else {
        error!("Unable to establish publisher");
        return;
    };

    let publisher_topic = UUri {
        authority_name: authority_name.to_string(),
        ue_id: ue_id as u32,
        ue_version_major,
        resource_id,
        ..Default::default()
    };

    loop {
        let publish_msg_res = UMessageBuilder::publish(publisher_topic.clone()).build();

        let Ok(publish_msg) = publish_msg_res else {
            error!(
                "Unable to create Publish UMessage: {:?}",
                publish_msg_res.err().unwrap()
            );
            continue;
        };

        let send_res = client.send(publish_msg).await;

        if let Err(err) = send_res {
            error!("Unable to send Publish UMessage: {:?}", err);
            continue;
        }

        tokio::time::sleep(Duration::from_millis(2000)).await;
    }
}
