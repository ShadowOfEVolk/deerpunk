use hecs::World;

pub trait System: Send {
    fn run(&mut self, world: &mut World, delta_time: f32);
}

impl<F: FnMut(&mut World, f32) + Send> System for F {
    fn run(&mut self, world: &mut World, delta_time: f32) {
        self(world, delta_time);
    }
}

pub struct Schedule {
    systems: Vec<Box<dyn System>>,
}

impl Schedule {
    pub fn new() -> Self {
        Self { systems: Vec::new() }
    }

    pub fn add_system<S: System + 'static>(&mut self, system: S) -> &mut Self {
        self.systems.push(Box::new(system));
        self
    }

    pub fn clear(&mut self) {
        self.systems.clear();
    }

    pub fn run(&mut self, world: &mut World, delta_time: f32) {
        for system in &mut self.systems {
            system.run(world, delta_time);
        }
    }
}

impl Default for Schedule {
    fn default() -> Self {
        Self::new()
    }
}
