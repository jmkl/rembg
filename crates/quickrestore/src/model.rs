pub enum Status {
    Idle,
    Processing,
    Error,
}

#[macro_export]
macro_rules! ps {
    ($proc:expr,$stat:expr) => {
        ProgressStatus::new($proc, $stat)
    };
    ($stat:expr) => {
        ProgressStatus::new(Status::Error, $stat)
    };
}

pub struct ProgressStatus {
    pub is_processing: Status,
    pub status: String,
}
impl ProgressStatus {
    pub fn new(is_processing: Status, status: impl Into<String>) -> Self {
        Self {
            is_processing,
            status: status.into(),
        }
    }
    pub fn is_processing(&self) -> bool {
        match self.is_processing {
            Status::Processing => true,
            _ => false,
        }
    }
}
pub enum Cmd {
    SendText(String),
}
