//! Streamed construction and deterministic factory state machines.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::economy::{DemandId, ResourceAllocation, ResourceDemand, Resources, TeamEconomy};
use crate::model::{UnitDef, Vec2};
use crate::placement::{BUILD_GRID_SIZE, BuildFacing, Yardmap, YardmapStatus};

pub type ProjectId = DemandId;
pub type BuilderId = u64;
pub type FactoryId = u64;

/// Deterministic build-pad and exit points for a factory product.
///
/// Yardmaps are interpreted in their definition's south-facing orientation
/// and then rotated with the factory. Explicit `e` cells take precedence for
/// the exit lane, while `c`/`i` yard cells identify the build pad. Definitions
/// without a usable yardmap use the footprint center and the facing edge. In
/// particular, the legacy/default south-facing fallback exits toward positive
/// world Z.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FactoryExitGeometry {
    pub pad: Vec2,
    pub goal: Vec2,
    pub direction: Vec2,
}

pub fn factory_exit_geometry(
    factory_position: Vec2,
    footprint: [u16; 2],
    facing: BuildFacing,
    yardmap: Option<&str>,
    product_radius: f32,
) -> FactoryExitGeometry {
    let [cols, rows] = facing.rotated_footprint(footprint);
    let direction = match facing {
        BuildFacing::South => Vec2::new(0.0, 1.0),
        BuildFacing::East => Vec2::new(1.0, 0.0),
        BuildFacing::North => Vec2::new(0.0, -1.0),
        BuildFacing::West => Vec2::new(-1.0, 0.0),
    };
    let lateral = Vec2::new(direction.z, -direction.x);
    let rotated = yardmap
        .and_then(|raw| Yardmap::parse(raw, footprint[0], footprint[1]).ok())
        .map(|parsed| facing.rotate_yardmap(&parsed));

    let mut yards = Vec::new();
    let mut exits = Vec::new();
    if let Some(yardmap) = &rotated {
        for row in 0..yardmap.rows {
            for col in 0..yardmap.cols {
                let local = Vec2::new(
                    (col as f32 + 0.5 - yardmap.cols as f32 * 0.5) * BUILD_GRID_SIZE,
                    (row as f32 + 0.5 - yardmap.rows as f32 * 0.5) * BUILD_GRID_SIZE,
                );
                match yardmap.get(col, row) {
                    Some(YardmapStatus::ExitOnly) => exits.push(local),
                    Some(YardmapStatus::Yard | YardmapStatus::YardInv) => yards.push(local),
                    _ => {}
                }
            }
        }
    }

    let pad_local = centroid(if yards.is_empty() { &exits } else { &yards });
    let exit_candidates = if exits.is_empty() { &yards } else { &exits };
    let exit_lateral = outward_lane(exit_candidates, direction, lateral)
        .unwrap_or(pad_local.x * lateral.x + pad_local.z * lateral.z);
    let boundary = if direction.x != 0.0 {
        cols as f32 * BUILD_GRID_SIZE * 0.5
    } else {
        rows as f32 * BUILD_GRID_SIZE * 0.5
    };
    let goal = factory_position
        + lateral * exit_lateral
        + direction * (boundary + product_radius.max(0.0) + 0.01);

    FactoryExitGeometry {
        pad: factory_position + pad_local,
        goal,
        direction,
    }
}

fn centroid(points: &[Vec2]) -> Vec2 {
    if points.is_empty() {
        return Vec2::default();
    }
    let sum = points
        .iter()
        .copied()
        .fold(Vec2::default(), |sum, point| sum + point);
    sum * (1.0 / points.len() as f32)
}

fn outward_lane(points: &[Vec2], direction: Vec2, lateral: Vec2) -> Option<f32> {
    let outward = points
        .iter()
        .map(|point| point.x * direction.x + point.z * direction.z)
        .reduce(f32::max)?;
    let mut lateral_sum = 0.0;
    let mut count = 0_u32;
    for point in points {
        let projection = point.x * direction.x + point.z * direction.z;
        if (projection - outward).abs() <= 1e-4 {
            lateral_sum += point.x * lateral.x + point.z * lateral.z;
            count += 1;
        }
    }
    (count > 0).then_some(lateral_sum / count as f32)
}

#[derive(Clone, Debug, PartialEq)]
pub struct WallSpec {
    /// Footprint in build-grid squares.
    pub footprint: [u16; 2],
    pub full_height: f32,
    pub blocks_movement: bool,
    pub blocks_fire: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ProjectKind {
    Unit { definition: String },
    Wall(WallSpec),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProjectSpec {
    pub kind: ProjectKind,
    pub cost: Resources,
    /// The work denominator used by Recoil-style `build_power / build_time`.
    pub build_time: f64,
    pub max_health: f64,
}

impl ProjectSpec {
    pub fn from_unit(definition: &UnitDef) -> Self {
        Self {
            kind: ProjectKind::Unit {
                definition: definition.name.clone(),
            },
            cost: Resources::new(definition.metal_cost as f64, definition.energy_cost as f64),
            build_time: definition.strategic.build_time as f64,
            max_health: definition.health as f64,
        }
    }

    /// Marks a real catalog unit as wall-like while retaining its actual cost,
    /// build time, health, footprint, and collision height.
    pub fn from_wall_unit(definition: &UnitDef) -> Self {
        Self {
            kind: ProjectKind::Wall(WallSpec {
                footprint: definition.strategic.footprint,
                full_height: definition.movement.collision_height(),
                blocks_movement: definition.movement.blocks_movement,
                blocks_fire: definition.movement.blocks_fire,
            }),
            cost: Resources::new(definition.metal_cost as f64, definition.energy_cost as f64),
            build_time: definition.strategic.build_time as f64,
            max_health: definition.health as f64,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectState {
    Queued,
    Nanoframe,
    Complete,
    Cancelled,
    Destroyed,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ConstructionProject {
    pub id: ProjectId,
    pub spec: ProjectSpec,
    pub priority: i32,
    pub progress: f64,
    pub health: f64,
    pub state: ProjectState,
}

impl ConstructionProject {
    pub fn queued(id: ProjectId, spec: ProjectSpec, priority: i32) -> Self {
        Self {
            id,
            spec,
            priority,
            progress: 0.0,
            health: 0.0,
            state: ProjectState::Queued,
        }
    }

    /// Places the nanoframe. From this point it is physical and attackable.
    pub fn place(&mut self) -> bool {
        if self.state != ProjectState::Queued {
            return false;
        }
        self.state = ProjectState::Nanoframe;
        true
    }

    pub fn nanoframe(id: ProjectId, spec: ProjectSpec, priority: i32) -> Self {
        let mut project = Self::queued(id, spec, priority);
        project.place();
        project
    }

    pub fn is_physical(&self) -> bool {
        matches!(self.state, ProjectState::Nanoframe | ProjectState::Complete)
    }

    pub fn blocks_movement(&self) -> bool {
        self.is_physical()
            && matches!(&self.spec.kind, ProjectKind::Wall(wall) if wall.blocks_movement)
    }

    pub fn blocks_fire(&self) -> bool {
        self.is_physical() && matches!(&self.spec.kind, ProjectKind::Wall(wall) if wall.blocks_fire)
    }

    /// Current wall height under the initial linear-growth approximation.
    pub fn collision_height(&self) -> Option<f32> {
        match &self.spec.kind {
            ProjectKind::Wall(wall) if self.is_physical() => {
                Some(wall.full_height * self.progress as f32)
            }
            _ => None,
        }
    }

    pub fn maximum_health_now(&self) -> f64 {
        self.spec.max_health.max(0.0) * self.progress
    }

    pub fn take_damage(&mut self, damage: f64) {
        if !self.is_physical() {
            return;
        }
        self.health = (self.health - damage.max(0.0)).max(0.0);
        if self.health <= f64::EPSILON {
            self.state = ProjectState::Destroyed;
        }
    }

    pub fn cancel(&mut self) {
        if matches!(self.state, ProjectState::Queued | ProjectState::Nanoframe) {
            self.state = ProjectState::Cancelled;
        }
    }

    fn advance(&mut self, progress: f64) -> f64 {
        if self.state != ProjectState::Nanoframe {
            return 0.0;
        }
        let applied = progress.max(0.0).min(1.0 - self.progress);
        self.progress += applied;
        self.health =
            (self.health + self.spec.max_health.max(0.0) * applied).min(self.maximum_health_now());
        if self.progress >= 1.0 - 1e-12 {
            self.progress = 1.0;
            self.state = ProjectState::Complete;
        }
        applied
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BuilderContribution {
    pub builder_id: BuilderId,
    pub project_id: ProjectId,
    pub build_power: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ConstructionReceipt {
    pub project_id: ProjectId,
    pub requested_progress: f64,
    pub applied_progress: f64,
    pub allocation_fraction: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ConstructionTick {
    pub receipts: Vec<ConstructionReceipt>,
    pub completed: Vec<ProjectId>,
    pub stalled_build_power_seconds: f64,
}

/// Immutable work request for the allocation phase of one tick. Its public
/// demands may be combined with upkeep, repair, and weapon demands before the
/// resulting allocations are returned to `apply_allocations`.
#[derive(Clone, Debug, Default)]
pub struct ConstructionPlan {
    pub demands: Vec<ResourceDemand>,
    seconds: f64,
    requests: BTreeMap<ProjectId, (f64, f64)>,
}

#[derive(Clone, Debug, Default)]
pub struct ConstructionSystem {
    projects: BTreeMap<ProjectId, ConstructionProject>,
}

impl ConstructionSystem {
    pub fn insert(&mut self, project: ConstructionProject) -> bool {
        self.projects.insert(project.id, project).is_none()
    }

    pub fn get(&self, id: ProjectId) -> Option<&ConstructionProject> {
        self.projects.get(&id)
    }

    pub fn get_mut(&mut self, id: ProjectId) -> Option<&mut ConstructionProject> {
        self.projects.get_mut(&id)
    }

    pub fn remove(&mut self, id: ProjectId) -> Option<ConstructionProject> {
        self.projects.remove(&id)
    }

    pub fn projects(&self) -> impl Iterator<Item = &ConstructionProject> {
        self.projects.values()
    }

    /// Gathers simultaneous work requests. Duplicate builder IDs are ignored
    /// after their first `(builder_id, project_id)` in stable order.
    pub fn plan(&self, seconds: f64, contributions: &[BuilderContribution]) -> ConstructionPlan {
        let seconds = if seconds.is_finite() {
            seconds.max(0.0)
        } else {
            0.0
        };
        let mut ordered = contributions.to_vec();
        ordered.sort_by_key(|contribution| (contribution.builder_id, contribution.project_id));
        let mut seen_builders = BTreeSet::new();
        let mut power_by_project = BTreeMap::<ProjectId, f64>::new();
        for contribution in ordered {
            if !seen_builders.insert(contribution.builder_id) {
                continue;
            }
            let power = if contribution.build_power.is_finite() {
                contribution.build_power.max(0.0)
            } else {
                0.0
            };
            *power_by_project.entry(contribution.project_id).or_default() += power;
        }

        let mut plan = ConstructionPlan {
            seconds,
            ..Default::default()
        };
        for (project_id, build_power) in power_by_project {
            let Some(project) = self.projects.get(&project_id) else {
                continue;
            };
            if project.state != ProjectState::Nanoframe {
                continue;
            }
            let build_time = project.spec.build_time.max(f64::EPSILON);
            let requested_progress = ((build_power / build_time) * seconds)
                .max(0.0)
                .min(1.0 - project.progress);
            if requested_progress <= f64::EPSILON {
                continue;
            }
            plan.requests
                .insert(project_id, (requested_progress, build_power));
            plan.demands.push(ResourceDemand {
                id: project_id,
                priority: project.priority,
                requested: project.spec.cost.scaled(requested_progress),
            });
        }

        plan
    }

    /// Applies allocations produced from this plan. Allocations for unrelated
    /// systems are ignored; a missing project allocation represents a full
    /// stall and applies no progress.
    pub fn apply_allocations(
        &mut self,
        plan: ConstructionPlan,
        allocations: &[ResourceAllocation],
    ) -> ConstructionTick {
        let fractions: BTreeMap<_, _> = allocations
            .iter()
            .map(|allocation| (allocation.id, allocation.fraction))
            .collect();
        let mut tick = ConstructionTick::default();
        for (project_id, (requested_progress, build_power)) in plan.requests {
            let allocation_fraction = fractions.get(&project_id).copied().unwrap_or(0.0);
            let project = self
                .projects
                .get_mut(&project_id)
                .expect("a planned project must remain live through allocation");
            let was_complete = project.state == ProjectState::Complete;
            let applied_progress = project.advance(requested_progress * allocation_fraction);
            tick.stalled_build_power_seconds +=
                build_power * plan.seconds * (1.0 - allocation_fraction);
            tick.receipts.push(ConstructionReceipt {
                project_id,
                requested_progress,
                applied_progress,
                allocation_fraction,
            });
            if !was_complete && project.state == ProjectState::Complete {
                tick.completed.push(project.id);
            }
        }
        tick
    }

    /// Convenience wrapper when construction is the only demand source.
    pub fn tick(
        &mut self,
        seconds: f64,
        economy: &mut TeamEconomy,
        contributions: &[BuilderContribution],
    ) -> ConstructionTick {
        let plan = self.plan(seconds, contributions);
        let allocations = economy.allocate(&plan.demands, seconds);
        self.apply_allocations(plan, &allocations)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct BuildItem {
    pub definition: String,
    pub project: ProjectSpec,
}

impl BuildItem {
    pub fn from_unit(definition: &UnitDef) -> Self {
        Self {
            definition: definition.name.clone(),
            project: ProjectSpec::from_unit(definition),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum FactoryState {
    Idle,
    Starting {
        item: BuildItem,
    },
    Constructing {
        item: BuildItem,
        project_id: ProjectId,
    },
    Releasing {
        item: BuildItem,
        project_id: ProjectId,
    },
    ExitBlocked {
        item: BuildItem,
        project_id: ProjectId,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum FactoryEvent {
    Starting {
        definition: String,
    },
    ProjectPlaced {
        definition: String,
        project_id: ProjectId,
    },
    ExitBecameBlocked {
        definition: String,
        project_id: ProjectId,
    },
    Released {
        definition: String,
        project_id: ProjectId,
    },
}

#[derive(Clone, Debug)]
pub struct Factory {
    pub id: FactoryId,
    pub build_power: f64,
    pub priority: i32,
    pub repeat: bool,
    pub state: FactoryState,
    queue: VecDeque<BuildItem>,
}

impl Factory {
    pub fn new(id: FactoryId, build_power: f64, priority: i32) -> Self {
        Self {
            id,
            build_power: build_power.max(0.0),
            priority,
            repeat: false,
            state: FactoryState::Idle,
            queue: VecDeque::new(),
        }
    }

    pub fn queue(&self) -> &VecDeque<BuildItem> {
        &self.queue
    }

    pub fn append(&mut self, item: BuildItem) {
        self.queue.push_back(item);
    }

    pub fn prepend(&mut self, item: BuildItem) {
        self.queue.push_front(item);
    }

    pub fn replace_queue(&mut self, items: impl IntoIterator<Item = BuildItem>) {
        self.queue = items.into_iter().collect();
    }

    pub fn cancel_queued(&mut self, index: usize) -> Option<BuildItem> {
        self.queue.remove(index)
    }

    pub fn clear_queue(&mut self) {
        self.queue.clear();
    }

    pub fn cancel_current(&mut self, construction: &mut ConstructionSystem) -> Option<BuildItem> {
        let previous = std::mem::replace(&mut self.state, FactoryState::Idle);
        match previous {
            FactoryState::Idle => None,
            FactoryState::Starting { item } => Some(item),
            FactoryState::Constructing { item, project_id } => {
                if let Some(project) = construction.get_mut(project_id) {
                    project.cancel();
                }
                Some(item)
            }
            state @ (FactoryState::Releasing { .. } | FactoryState::ExitBlocked { .. }) => {
                self.state = state;
                None
            }
        }
    }

    /// Runs state transitions that happen before resource allocation. The
    /// caller supplies globally unique project IDs and the current yard state.
    pub fn pre_construction(
        &mut self,
        construction: &mut ConstructionSystem,
        exit_clear: bool,
        next_project_id: impl FnOnce() -> ProjectId,
    ) -> Vec<FactoryEvent> {
        let previous = std::mem::replace(&mut self.state, FactoryState::Idle);
        let mut events = Vec::new();
        self.state = match previous {
            FactoryState::Idle => {
                if let Some(item) = self.queue.pop_front() {
                    events.push(FactoryEvent::Starting {
                        definition: item.definition.clone(),
                    });
                    FactoryState::Starting { item }
                } else {
                    FactoryState::Idle
                }
            }
            FactoryState::Starting { item } => {
                let project_id = next_project_id();
                let inserted = construction.insert(ConstructionProject::nanoframe(
                    project_id,
                    item.project.clone(),
                    self.priority,
                ));
                if inserted {
                    events.push(FactoryEvent::ProjectPlaced {
                        definition: item.definition.clone(),
                        project_id,
                    });
                    FactoryState::Constructing { item, project_id }
                } else {
                    FactoryState::Starting { item }
                }
            }
            FactoryState::Constructing { item, project_id } => {
                FactoryState::Constructing { item, project_id }
            }
            FactoryState::Releasing { item, project_id } => {
                if exit_clear {
                    self.finish_release(item, project_id, construction, &mut events)
                } else {
                    events.push(FactoryEvent::ExitBecameBlocked {
                        definition: item.definition.clone(),
                        project_id,
                    });
                    FactoryState::ExitBlocked { item, project_id }
                }
            }
            FactoryState::ExitBlocked { item, project_id } => {
                if exit_clear {
                    self.finish_release(item, project_id, construction, &mut events)
                } else {
                    FactoryState::ExitBlocked { item, project_id }
                }
            }
        };
        events
    }

    pub fn builder_contribution(&self) -> Option<BuilderContribution> {
        let FactoryState::Constructing { project_id, .. } = self.state else {
            return None;
        };
        Some(BuilderContribution {
            builder_id: self.id,
            project_id,
            build_power: self.build_power,
        })
    }

    /// Observes simultaneous construction results after allocation. Release is
    /// intentionally deferred to the next pre-construction boundary.
    pub fn post_construction(&mut self, construction: &ConstructionSystem) {
        let FactoryState::Constructing { item, project_id } = &self.state else {
            return;
        };
        if construction
            .get(*project_id)
            .is_some_and(|project| project.state == ProjectState::Complete)
        {
            self.state = FactoryState::Releasing {
                item: item.clone(),
                project_id: *project_id,
            };
        }
    }

    fn finish_release(
        &mut self,
        item: BuildItem,
        project_id: ProjectId,
        construction: &mut ConstructionSystem,
        events: &mut Vec<FactoryEvent>,
    ) -> FactoryState {
        construction.remove(project_id);
        if self.repeat {
            self.queue.push_back(item.clone());
        }
        events.push(FactoryEvent::Released {
            definition: item.definition,
            project_id,
        });
        FactoryState::Idle
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::economy::ResourcePool;

    fn team(metal: f64, energy: f64) -> TeamEconomy {
        TeamEconomy::new(
            ResourcePool::new(metal, 10_000.0, 0.0),
            ResourcePool::new(energy, 10_000.0, 0.0),
        )
    }

    fn wall_project(id: ProjectId) -> ConstructionProject {
        ConstructionProject::nanoframe(
            id,
            ProjectSpec {
                kind: ProjectKind::Wall(WallSpec {
                    footprint: [1, 1],
                    full_height: 40.0,
                    blocks_movement: true,
                    blocks_fire: true,
                }),
                cost: Resources::new(100.0, 0.0),
                build_time: 100.0,
                max_health: 1_000.0,
            },
            0,
        )
    }

    fn build_item(name: &str) -> BuildItem {
        BuildItem {
            definition: name.into(),
            project: ProjectSpec {
                kind: ProjectKind::Unit {
                    definition: name.into(),
                },
                cost: Resources::new(100.0, 0.0),
                build_time: 100.0,
                max_health: 500.0,
            },
        }
    }

    #[test]
    fn resource_limited_projects_advance_pro_rata() {
        let mut construction = ConstructionSystem::default();
        construction.insert(wall_project(1));
        construction.insert(wall_project(2));
        let mut economy = team(50.0, 0.0);
        let tick = construction.tick(
            1.0,
            &mut economy,
            &[
                BuilderContribution {
                    builder_id: 1,
                    project_id: 1,
                    build_power: 100.0,
                },
                BuilderContribution {
                    builder_id: 2,
                    project_id: 2,
                    build_power: 100.0,
                },
            ],
        );
        assert_eq!(construction.get(1).unwrap().progress, 0.25);
        assert_eq!(construction.get(2).unwrap().progress, 0.25);
        assert_eq!(tick.stalled_build_power_seconds, 150.0);
    }

    #[test]
    fn construction_plan_combines_with_higher_priority_upkeep() {
        let mut construction = ConstructionSystem::default();
        construction.insert(wall_project(1));
        let mut economy = team(100.0, 0.0);
        let plan = construction.plan(
            1.0,
            &[BuilderContribution {
                builder_id: 1,
                project_id: 1,
                build_power: 100.0,
            }],
        );
        let mut all_demands = plan.demands.clone();
        all_demands.push(ResourceDemand::from_rate(
            900,
            100,
            Resources::new(75.0, 0.0),
            1.0,
        ));
        let allocations = economy.allocate(&all_demands, 1.0);
        construction.apply_allocations(plan, &allocations);
        assert_eq!(construction.get(1).unwrap().progress, 0.25);
        assert_eq!(economy.metal.stored, 0.0);
    }

    #[test]
    fn wall_is_physical_at_placement_and_grows_with_progress() {
        let mut construction = ConstructionSystem::default();
        construction.insert(wall_project(7));
        assert!(construction.get(7).unwrap().blocks_movement());
        assert_eq!(construction.get(7).unwrap().collision_height(), Some(0.0));

        let mut economy = team(50.0, 0.0);
        construction.tick(
            1.0,
            &mut economy,
            &[BuilderContribution {
                builder_id: 1,
                project_id: 7,
                build_power: 50.0,
            }],
        );
        let wall = construction.get(7).unwrap();
        assert_eq!(wall.progress, 0.5);
        assert_eq!(wall.collision_height(), Some(20.0));
        assert_eq!(wall.health, 500.0);
    }

    #[test]
    fn factory_waits_for_exit_and_preserves_repeat_queue() {
        let mut construction = ConstructionSystem::default();
        let mut factory = Factory::new(42, 100.0, 0);
        factory.repeat = true;
        factory.append(build_item("pawn"));

        factory.pre_construction(&mut construction, true, || 100);
        factory.pre_construction(&mut construction, true, || 100);
        let contribution = factory.builder_contribution().unwrap();
        let mut economy = team(100.0, 0.0);
        construction.tick(1.0, &mut economy, &[contribution]);
        factory.post_construction(&construction);
        assert!(matches!(factory.state, FactoryState::Releasing { .. }));

        let blocked = factory.pre_construction(&mut construction, false, || 101);
        assert!(matches!(factory.state, FactoryState::ExitBlocked { .. }));
        assert!(matches!(
            blocked.as_slice(),
            [FactoryEvent::ExitBecameBlocked { .. }]
        ));
        assert!(construction.get(100).is_some());

        let released = factory.pre_construction(&mut construction, true, || 101);
        assert!(matches!(
            released.as_slice(),
            [FactoryEvent::Released { .. }]
        ));
        assert!(matches!(factory.state, FactoryState::Idle));
        assert!(construction.get(100).is_none());
        assert_eq!(factory.queue().len(), 1);
    }

    #[test]
    fn factory_exit_geometry_uses_facing_fallback() {
        let south =
            factory_exit_geometry(Vec2::new(64.0, 64.0), [2, 4], BuildFacing::South, None, 4.0);
        assert_eq!(south.pad, Vec2::new(64.0, 64.0));
        assert_eq!(south.direction, Vec2::new(0.0, 1.0));
        assert!(south.goal.z > 96.0);

        let east =
            factory_exit_geometry(Vec2::new(64.0, 64.0), [2, 4], BuildFacing::East, None, 4.0);
        assert_eq!(east.direction, Vec2::new(1.0, 0.0));
        assert!(east.goal.x > 96.0);
    }

    #[test]
    fn factory_exit_geometry_uses_rotated_yard_and_exit_lane() {
        // South-facing source: build pad on the left and an explicit exit on
        // the right/front. East rotation must carry both points with it.
        let geometry = factory_exit_geometry(
            Vec2::new(64.0, 64.0),
            [2, 2],
            BuildFacing::East,
            Some("oc oe"),
            4.0,
        );
        assert_eq!(geometry.direction, Vec2::new(1.0, 0.0));
        assert_ne!(geometry.pad, Vec2::new(64.0, 64.0));
        assert!(geometry.goal.x > 80.0);
    }

    #[test]
    fn duplicate_builder_assignment_is_resolved_by_project_id() {
        let mut construction = ConstructionSystem::default();
        construction.insert(wall_project(1));
        construction.insert(wall_project(2));
        let mut economy = team(100.0, 0.0);
        construction.tick(
            1.0,
            &mut economy,
            &[
                BuilderContribution {
                    builder_id: 9,
                    project_id: 2,
                    build_power: 100.0,
                },
                BuilderContribution {
                    builder_id: 9,
                    project_id: 1,
                    build_power: 100.0,
                },
            ],
        );
        assert_eq!(construction.get(1).unwrap().progress, 1.0);
        assert_eq!(construction.get(2).unwrap().progress, 0.0);
    }
}
