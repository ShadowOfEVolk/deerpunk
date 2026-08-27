pub fn short_type_name<T: ?Sized>() -> &'static str {
    let full = std::any::type_name::<T>();
    full.split("::").last().unwrap_or(full)
}
