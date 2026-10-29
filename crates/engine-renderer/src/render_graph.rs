use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::c_void;

use wgpu::TextureFormat;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ResId(pub &'static str);

#[derive(Debug, Clone)]
pub enum Resource {
    External,
    Texture {
        format: TextureFormat,
        width: u32,
        height: u32,
    },
}

pub struct TexturePool {
    free: Vec<wgpu::Texture>,
}

impl TexturePool {
    pub fn new() -> Self {
        Self { free: Vec::new() }
    }

    pub fn acquire(
        &mut self,
        device: &wgpu::Device,
        format: TextureFormat,
        width: u32,
        height: u32,
    ) -> wgpu::Texture {
        let idx = self.free.iter().position(|t| {
            t.format() == format && t.width() >= width && t.height() >= height
        });

        match idx {
            Some(i) => self.free.swap_remove(i),
            None => device.create_texture(&wgpu::TextureDescriptor {
                label: Some("render_graph_pool"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            }),
        }
    }

    pub fn release(&mut self, texture: wgpu::Texture) {
        self.free.push(texture);
    }

    pub fn clear(&mut self) {
        self.free.clear();
    }
}

impl Default for TexturePool {
    fn default() -> Self {
        Self::new()
    }
}

pub struct NodeContext<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub encoder: &'a mut wgpu::CommandEncoder,
    pub inputs: HashMap<ResId, &'a wgpu::TextureView>,
    pub outputs: HashMap<ResId, &'a wgpu::TextureView>,
    /// Per-frame state for pass dispatch (`GraphFrameUserData` in forward graph).
    pub user_data: *mut c_void,
    pub node_name: &'static str,
}

pub type NodeExecute = Box<dyn Fn(&mut NodeContext<'_>) + Send + Sync>;

pub struct Node {
    pub name: &'static str,
    pub inputs: Vec<ResId>,
    pub outputs: Vec<ResId>,
    pub execute: NodeExecute,
}

pub struct RenderGraph {
    nodes: Vec<Node>,
    resources: HashMap<ResId, Resource>,
    pub input: ResId,
    pub output: ResId,
    texture_pool: TexturePool,
}

impl RenderGraph {
    pub fn new(input: ResId, output: ResId) -> Self {
        Self {
            nodes: Vec::new(),
            resources: HashMap::new(),
            input,
            output,
            texture_pool: TexturePool::new(),
        }
    }

    pub fn add_resource(mut self, id: ResId, res: Resource) -> Self {
        self.resources.insert(id, res);
        self
    }

    pub fn add_node(mut self, node: Node) -> Self {
        self.nodes.push(node);
        self
    }

    pub fn compile(&self) -> Result<Vec<usize>, GraphError> {
        let n = self.nodes.len();
        let mut in_degree = vec![0u32; n];
        let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];

        let mut seen_names: HashSet<&str> = HashSet::new();
        for node in &self.nodes {
            if !seen_names.insert(node.name) {
                return Err(GraphError::DuplicateNode(node.name));
            }
        }

        for (i, node) in self.nodes.iter().enumerate() {
            for input_id in &node.inputs {
                if !self.resources.contains_key(input_id) {
                    return Err(GraphError::UnboundResource(*input_id));
                }
                for (j, other) in self.nodes.iter().enumerate() {
                    if i != j && other.outputs.contains(input_id) {
                        adj[j].push(i);
                        in_degree[i] += 1;
                    }
                }
            }
            for output_id in &node.outputs {
                if !self.resources.contains_key(output_id) {
                    return Err(GraphError::UnboundResource(*output_id));
                }
            }
        }

        let mut queue: VecDeque<usize> = VecDeque::new();
        for i in 0..n {
            if in_degree[i] == 0 {
                queue.push_back(i);
            }
        }

        let mut order = Vec::with_capacity(n);
        while let Some(u) = queue.pop_front() {
            order.push(u);
            for &v in &adj[u] {
                in_degree[v] -= 1;
                if in_degree[v] == 0 {
                    queue.push_back(v);
                }
            }
        }

        if order.len() != n {
            return Err(GraphError::Cycle);
        }

        Ok(order)
    }

    /// Record graph nodes into `encoder` without submitting (topological order from `compile()`).
    pub fn execute_on_encoder(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        external: &HashMap<ResId, wgpu::TextureView>,
        user_data: *mut c_void,
    ) -> Result<Vec<&'static str>, GraphError> {
        let order = self.compile()?;
        self.execute_on_encoder_with_order(
            device,
            queue,
            encoder,
            external,
            user_data,
            &order,
        )
    }

    /// Forward / aliased-external frames: run nodes in **registration order** (no topological sort).
    ///
    /// When several nodes read/write the same [`Resource::External`] id (e.g. one HDR target),
    /// `compile()` would see false cycles (sprites → mesh → sprites). Use this for that case.
    pub fn execute_on_encoder_in_order(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        external: &HashMap<ResId, wgpu::TextureView>,
        user_data: *mut c_void,
    ) -> Result<Vec<&'static str>, GraphError> {
        let order: Vec<usize> = (0..self.nodes.len()).collect();
        self.execute_on_encoder_with_order(
            device,
            queue,
            encoder,
            external,
            user_data,
            &order,
        )
    }

    fn execute_on_encoder_with_order(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        external: &HashMap<ResId, wgpu::TextureView>,
        user_data: *mut c_void,
        order: &[usize],
    ) -> Result<Vec<&'static str>, GraphError> {
        let mut acquired: Vec<(ResId, wgpu::Texture)> = Vec::new();
        let mut view_storage: HashMap<ResId, wgpu::TextureView> = HashMap::new();

        for (id, view) in external {
            if self.resources.contains_key(id) {
                view_storage.insert(*id, view.clone());
            }
        }

        let mut executed = Vec::with_capacity(order.len());
        for &node_idx in order {
            let node = &self.nodes[node_idx];
            executed.push(node.name);

            for output_id in &node.outputs {
                if view_storage.contains_key(output_id) {
                    continue;
                }
                if let Some(view) = external.get(output_id) {
                    view_storage.insert(*output_id, view.clone());
                    continue;
                }
                if let Some(Resource::Texture {
                    format,
                    width,
                    height,
                }) = self.resources.get(output_id)
                {
                    let tex = self.texture_pool.acquire(device, *format, *width, *height);
                    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
                    view_storage.insert(*output_id, view);
                    acquired.push((*output_id, tex));
                }
            }

            for input_id in &node.inputs {
                if view_storage.contains_key(input_id) {
                    continue;
                }
                if let Some(view) = external.get(input_id) {
                    view_storage.insert(*input_id, view.clone());
                } else {
                    return Err(GraphError::MissingTexture(*input_id));
                }
            }

            let input_views: HashMap<ResId, &wgpu::TextureView> = node
                .inputs
                .iter()
                .filter_map(|id| view_storage.get(id).map(|v| (*id, v)))
                .collect();

            let mut output_views: HashMap<ResId, &wgpu::TextureView> = HashMap::new();
            for output_id in &node.outputs {
                if let Some(view) = view_storage.get(output_id) {
                    output_views.insert(*output_id, view);
                }
            }

            let mut ctx = NodeContext {
                device,
                queue,
                encoder,
                inputs: input_views,
                outputs: output_views,
                user_data,
                node_name: node.name,
            };

            (node.execute)(&mut ctx);
        }

        for (_, tex) in acquired {
            self.texture_pool.release(tex);
        }

        Ok(executed)
    }

    pub fn run(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_view: &wgpu::TextureView,
    ) -> Result<(), GraphError> {
        let mut external = HashMap::new();
        external.insert(self.input, surface_view.clone());
        external.insert(self.output, surface_view.clone());

        let mut encoder =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("render_graph"),
            });

        self.execute_on_encoder(
            device,
            queue,
            &mut encoder,
            &external,
            std::ptr::null_mut(),
        )?;

        queue.submit(std::iter::once(encoder.finish()));
        Ok(())
    }

    pub fn pool(&self) -> &TexturePool {
        &self.texture_pool
    }

    pub fn pool_mut(&mut self) -> &mut TexturePool {
        &mut self.texture_pool
    }

    /// Node names in compilation order (for HUD / inspector).
    pub fn node_names_ordered(&self) -> Result<Vec<&'static str>, GraphError> {
        let order = self.compile()?;
        Ok(order.iter().map(|&i| self.nodes[i].name).collect())
    }

    /// Static graph structure: `name -> (inputs, outputs)` resource ids.
    pub fn describe_nodes(&self) -> Vec<(&'static str, Vec<&'static str>, Vec<&'static str>)> {
        self.nodes
            .iter()
            .map(|n| {
                (
                    n.name,
                    n.inputs.iter().map(|r| r.0).collect(),
                    n.outputs.iter().map(|r| r.0).collect(),
                )
            })
            .collect()
    }
}

#[derive(Debug)]
pub enum GraphError {
    DuplicateNode(&'static str),
    UnboundResource(ResId),
    Cycle,
    MissingTexture(ResId),
}

impl std::fmt::Display for GraphError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GraphError::DuplicateNode(name) => write!(f, "duplicate node: {}", name),
            GraphError::UnboundResource(id) => {
                write!(f, "resource not declared: {}", id.0)
            }
            GraphError::Cycle => write!(f, "cycle detected in render graph"),
            GraphError::MissingTexture(id) => {
                write!(f, "texture not ready for node input: {}", id.0)
            }
        }
    }
}

impl std::error::Error for GraphError {}
