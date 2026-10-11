use crate::system_profiler;

pub trait System: Send {
    fn run(&mut self, delta_time: f32);
}

impl<F: FnMut(f32) + Send> System for F {
    fn run(&mut self, delta_time: f32) {
        self(delta_time);
    }
}

struct NamedSystem {
    name: &'static str,
    system: Box<dyn System>,
}

pub struct Schedule {
    systems: Vec<NamedSystem>,
}

impl Schedule {
    pub fn new() -> Self {
        Self {
            systems: Vec::new(),
        }
    }

    pub fn add_system<S: System + 'static>(&mut self, system: S) -> &mut Self {
        self.add_named("system", system)
    }

    pub fn add_named<S: System + 'static>(
        &mut self,
        name: &'static str,
        system: S,
    ) -> &mut Self {
        self.systems.push(NamedSystem {
            name,
            system: Box::new(system),
        });
        self
    }

    pub fn clear(&mut self) {
        self.systems.clear();
    }

    pub fn names(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.systems.iter().map(|s| s.name)
    }

    pub fn run(&mut self, delta_time: f32) {
        for entry in &mut self.systems {
            let _g = system_profiler::scope(entry.name);
            entry.system.run(delta_time);
        }
    }
}

impl Default for Schedule {
    fn default() -> Self {
        Self::new()
    }
}
