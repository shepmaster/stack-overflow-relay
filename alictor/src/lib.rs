use futures_channel::oneshot;
use snafu::Snafu;

pub use alictor_derive::alictor;

#[derive(Debug, Snafu)]
#[snafu(visibility(pub))] // TODO: eh... maybe wrap the oneshot to avoid leaking?
pub struct ActorError {
    source: oneshot::Canceled,
}

#[doc(hidden)]
pub mod reexport {
    pub mod futures {
        pub use futures_util::{sink::SinkExt, stream::StreamExt};

        pub mod channel {
            pub mod mpsc {
                pub use futures_channel::mpsc::{channel, Sender};
            }

            pub mod oneshot {
                pub use futures_channel::oneshot::{channel, Sender};
            }
        }

        pub mod executor {
            pub use futures_executor::block_on_stream;
        }
    }

    pub mod snafu {
        pub use snafu::ResultExt;
    }

    pub mod tokio {
        pub mod task {
            pub use tokio::task::{spawn, spawn_blocking, JoinHandle};
        }
    }
}
