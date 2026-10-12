// Network replication traits and types shared between client and server

pub struct NetworkField {
    pub offset: usize,
    pub size: usize,
}

pub trait NetworkComponent {
    fn declare_fields() -> &'static [NetworkField] where Self: Sized;
}

