use log::{error, trace};
use std::sync::Arc;
use std::time::Duration;
use up_client_vsomeip_rust::UPClientVsomeip;
use up_rust::{UListener, UMessage, UMessageBuilder, UStatus, UTransport, UUri};

pub struct SubscriberListener;
#[async_trait::async_trait]
impl UListener for SubscriberListener {
    async fn on_receive(&self, msg: UMessage) {
        println!("{:?}", msg);
    }

    async fn on_error(&self, err: UStatus) {
        println!("{:?}", err);
    }
}

#[tokio::test]
async fn publisher_subscriber() {
    env_logger::init();

    let app_name = "publisher";
    let authority_name = "foo";

    let ue_id = 10;
    let subscriber_ue_id = 20;
    let ue_version_major = 1;
    let resource_id = 0x8001;

    let publisher_topic = UUri {
        authority_name: authority_name.to_string(),
        ue_id: ue_id as u32,
        ue_version_major,
        resource_id,
        ..Default::default()
    };

    let subscriber_res = UPClientVsomeip::new(&authority_name.to_string(), subscriber_ue_id);

    let Ok(subscriber) = subscriber_res else {
        error!("Unable to establish subscriber");
        return;
    };

    let subscriber_listener_check = Arc::new(SubscriberListener);
    let subscriber_listener: Arc<dyn UListener> = subscriber_listener_check.clone();

    let reg_res = subscriber
        .register_listener(&publisher_topic, None, subscriber_listener)
        .await;

    if let Err(err) = reg_res {
        error!("Unable to register: {:?}", err);
    }

    let publisher_res = UPClientVsomeip::new(&authority_name.to_string(), ue_id);

    let Ok(publisher) = publisher_res else {
        error!("Unable to establish publisher");
        return;
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

        trace!("Publish message we're about to send:\n{publish_msg:?}");

        let send_res = publisher.send(publish_msg).await;

        if let Err(err) = send_res {
            error!("Unable to send Publish UMessage: {:?}", err);
            continue;
        }

        tokio::time::sleep(Duration::from_millis(2000)).await;
    }
}
