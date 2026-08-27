use std::collections::{HashMap, HashSet, VecDeque};

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

    pub fn run(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_view: &wgpu::TextureView,
    ) -> Result<(), GraphError> {
        let order = self.compile()?;
        let mut acquired: Vec<(ResId, wgpu::Texture)> = Vec::new();
        let mut view_storage: HashMap<ResId, wgpu::TextureView> = HashMap::new();

        let mut encoder =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("render_graph"),
            });

        for &node_idx in &order {
            let node = &self.nodes[node_idx];

            let input_data: Vec<(ResId, wgpu::TextureView)> = node.inputs.iter().map(|input_id| {
                if *input_id == self.input {
                    (*input_id, surface_view.clone())
                } else if let Some(view) = view_storage.get(input_id) {
                    (*input_id, view.clone())
                } else {
                    panic!("missing texture for input {}", input_id.0);
                }
            }).collect();

            let mut new_views: Vec<(ResId, wgpu::TextureView)> = Vec::new();
            for output_id in &node.outputs {
                if *output_id == self.output {
                    view_storage.insert(*output_id, surface_view.clone());
                } else if !view_storage.contains_key(output_id) {
                    let params = match self.resources.get(output_id) {
                        Some(Resource::Texture { format, width, height }) => Some((*format, *width, *height)),
                        _ => None,
                    };
                    let (format, width, height) = params
                        .ok_or(GraphError::MissingTexture(*output_id))?;
                    let tex =
                        self.texture_pool.acquire(device, format, width, height);
                    let view =
                        tex.create_view(&wgpu::TextureViewDescriptor::default());
                    new_views.push((*output_id, view));
                    acquired.push((*output_id, tex));
                }
            }
            for (id, view) in new_views {
                view_storage.insert(id, view);
            }

            let input_views: HashMap<ResId, &wgpu::TextureView> = input_data.iter()
                .map(|(id, view)| (*id, view))
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
                encoder: &mut encoder,
                inputs: input_views,
                outputs: output_views,
            };

            (node.execute)(&mut ctx);
        }

        queue.submit(std::iter::once(encoder.finish()));

        for (_, tex) in acquired {
            self.texture_pool.release(tex);
        }

        Ok(())
    }

    pub fn pool(&self) -> &TexturePool {
        &self.texture_pool
    }

    pub fn pool_mut(&mut self) -> &mut TexturePool {
        &mut self.texture_pool
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
