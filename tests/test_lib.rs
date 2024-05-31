use std::sync::Once;
use up_rust::{UListener, UMessage, UStatus};

static INIT: Once = Once::new();
pub fn before_test() {
    INIT.call_once(env_logger::init);
}

pub struct PrintingListener;
#[async_trait::async_trait]
impl UListener for PrintingListener {
    async fn on_receive(&self, msg: UMessage) {
        println!("{:?}", msg);
    }

    async fn on_error(&self, err: UStatus) {
        println!("{:?}", err);
    }
}
