//! Dependency-light HTML viewer and human-input protocol for simulator traces.

use crate::model::UnitId;
use crate::trace::{AggregateTrace, RunTrace};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "view", content = "data", rename_all = "snake_case")]
pub enum ViewerData {
    Run(RunTrace),
    Aggregate(AggregateTrace),
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FallbackSymbol {
    Circle,
    Triangle,
    Square,
    Diamond,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct IconStyle {
    /// Browser-readable URI, normally an embedded PNG/SVG data URI.
    pub uri: Option<String>,
    pub fallback: FallbackSymbol,
}

/// Resolves Recoil icon-type keys to BAR strategic icon bitmaps. Callers may
/// insert exact paths obtained from `gamedata/icontypes.lua`; otherwise common
/// BAR source-tree locations are probed and a stable geometric symbol is used.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct IconCatalog {
    icons: BTreeMap<String, IconStyle>,
}

impl IconCatalog {
    pub fn insert_uri(&mut self, key: impl Into<String>, uri: impl Into<String>) {
        let key = normalized_key(&key.into());
        let fallback = fallback_for(&key);
        self.icons.insert(
            key,
            IconStyle {
                uri: Some(uri.into()),
                fallback,
            },
        );
    }

    pub fn insert_file(&mut self, key: impl Into<String>, path: &Path) -> std::io::Result<()> {
        let mime = match path
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str()
        {
            "png" => "image/png",
            "svg" => "image/svg+xml",
            "webp" => "image/webp",
            _ => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "viewer supports PNG, SVG, and WebP icons",
                ));
            }
        };
        let encoded = base64(&fs::read(path)?);
        self.insert_uri(key, format!("data:{mime};base64,{encoded}"));
        Ok(())
    }

    pub fn resolve(&self, key: &str) -> IconStyle {
        let key = normalized_key(key);
        self.icons.get(&key).cloned().unwrap_or(IconStyle {
            uri: None,
            fallback: fallback_for(&key),
        })
    }

    /// Attempts conventional source-tree paths for icon keys. Explicit paths
    /// parsed from BAR's icon-type definitions should be preferred when known.
    pub fn probe_bar_root<I, S>(bar_root: &Path, keys: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut catalog = Self::default();
        for key in keys {
            let key = normalized_key(key.as_ref());
            if let Some(path) = icon_candidates(bar_root, &key)
                .into_iter()
                .find(|path| path.is_file())
            {
                let _ = catalog.insert_file(key, &path);
            }
        }
        catalog
    }
}

fn icon_candidates(root: &Path, key: &str) -> Vec<PathBuf> {
    let bases = [
        root.join("bitmaps/icons"),
        root.join("icons"),
        root.join("LuaUI/Images/icons"),
    ];
    let mut result = Vec::with_capacity(bases.len() * 3);
    for base in bases {
        for extension in ["png", "svg", "webp"] {
            result.push(base.join(format!("{key}.{extension}")));
        }
    }
    result
}

fn normalized_key(key: &str) -> String {
    key.trim().replace('\\', "/").to_ascii_lowercase()
}

fn fallback_for(key: &str) -> FallbackSymbol {
    let hash = key.bytes().fold(0_u32, |hash, byte| {
        hash.wrapping_mul(16_777_619) ^ u32::from(byte)
    });
    match hash & 3 {
        0 => FallbackSymbol::Circle,
        1 => FallbackSymbol::Triangle,
        2 => FallbackSymbol::Square,
        _ => FallbackSymbol::Diamond,
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct HumanCommand {
    /// Monotonic client sequence number for deterministic ordering/replay.
    pub sequence: u64,
    /// Simulation tick at which the command should first be eligible.
    pub apply_at_tick: u64,
    pub player: u8,
    pub unit_ids: Vec<UnitId>,
    pub action: HumanAction,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HumanAction {
    Move { position: [f32; 2], queued: bool },
    AttackUnit { target: UnitId, queued: bool },
    AttackGround { position: [f32; 2], queued: bool },
    Stop,
    SetPaused { paused: bool },
    Step { ticks: u32 },
    SetSpeed { multiplier: f32 },
}

/// Transport-neutral queue for a local UI, websocket adapter, or test bot.
/// Commands are drained at tick boundaries and sorted deterministically.
#[derive(Debug, Default)]
pub struct HumanCommandInbox {
    pending: VecDeque<HumanCommand>,
}

impl HumanCommandInbox {
    pub fn push(&mut self, command: HumanCommand) {
        self.pending.push_back(command);
    }

    pub fn drain_ready(&mut self, tick: u64) -> Vec<HumanCommand> {
        let mut ready = Vec::new();
        let mut waiting = VecDeque::with_capacity(self.pending.len());
        while let Some(command) = self.pending.pop_front() {
            if command.apply_at_tick <= tick {
                ready.push(command);
            } else {
                waiting.push_back(command);
            }
        }
        self.pending = waiting;
        ready.sort_by_key(|command| (command.apply_at_tick, command.player, command.sequence));
        ready
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ViewerDocument {
    pub title: String,
    pub data: ViewerData,
    pub icons: IconCatalog,
    /// Side controlled by canvas commands. None makes the viewer read-only.
    pub human_side: Option<u8>,
}

impl ViewerDocument {
    /// Produces one dependency-free HTML file. Run traces animate; aggregate
    /// traces show heat layers. The page emits `visigoth-command` DOM events
    /// and also appends command objects to `window.visigothCommands`.
    pub fn to_html(&self) -> Result<String, serde_json::Error> {
        let json = serde_json::to_string(self)?.replace('<', "\\u003c");
        Ok(HTML_TEMPLATE.replace("__VISIGOTH_DATA__", &json))
    }

    pub fn write_html(&self, path: &Path) -> Result<(), ViewerWriteError> {
        let html = self.to_html().map_err(ViewerWriteError::Serialize)?;
        fs::write(path, html).map_err(ViewerWriteError::Io)
    }
}

#[derive(Debug)]
pub enum ViewerWriteError {
    Serialize(serde_json::Error),
    Io(std::io::Error),
}

impl std::fmt::Display for ViewerWriteError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Serialize(error) => write!(formatter, "could not serialize viewer: {error}"),
            Self::Io(error) => write!(formatter, "could not write viewer: {error}"),
        }
    }
}

impl std::error::Error for ViewerWriteError {}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let a = chunk[0];
        let b = chunk.get(1).copied().unwrap_or(0);
        let c = chunk.get(2).copied().unwrap_or(0);
        output.push(TABLE[(a >> 2) as usize] as char);
        output.push(TABLE[(((a & 3) << 4) | (b >> 4)) as usize] as char);
        output.push(if chunk.len() > 1 {
            TABLE[(((b & 15) << 2) | (c >> 6)) as usize] as char
        } else {
            '='
        });
        output.push(if chunk.len() > 2 {
            TABLE[(c & 63) as usize] as char
        } else {
            '='
        });
    }
    output
}

const HTML_TEMPLATE: &str = r##"<!doctype html>
<html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width">
<title>Visigoth simulation</title>
<style>
html,body{margin:0;height:100%;background:#111;color:#ddd;font:14px system-ui}body{display:grid;grid-template-rows:auto 1fr}
#bar{padding:7px 10px;background:#202328;display:flex;gap:10px;align-items:center}canvas{width:100%;height:100%;display:block;cursor:crosshair}
button,input{accent-color:#7ab6ff}#status{margin-left:auto;color:#aaa}
</style></head><body><div id="bar"><strong id="title"></strong><button id="play">Pause</button>
<input id="time" type="range" min="0" value="0" step="1"><select id="layer"><option value="occupancy">Occupancy</option><option value="aim_points">Aim points</option><option value="deaths">Deaths</option></select><span id="status"></span></div>
<canvas id="view"></canvas><script>
const doc=__VISIGOTH_DATA__, canvas=document.querySelector('#view'), ctx=canvas.getContext('2d');
const slider=document.querySelector('#time'), play=document.querySelector('#play'), layer=document.querySelector('#layer');
document.querySelector('#title').textContent=doc.title; window.visigothCommands=[];
let frame=0,running=true,last=0,selected=[],sequence=0,images={};
for(const [key,style] of Object.entries(doc.icons.icons||{})){if(style.uri){const i=new Image();i.src=style.uri;images[key]=i;}}
function resize(){const d=devicePixelRatio||1,w=canvas.clientWidth,h=canvas.clientHeight;if(canvas.width!==w*d||canvas.height!==h*d){canvas.width=w*d;canvas.height=h*d;}ctx.setTransform(d,0,0,d,0,0);draw();}
function emit(action,ids=selected){const command={sequence:sequence++,apply_at_tick:currentTick()+1,player:doc.human_side,unit_ids:ids,action};window.visigothCommands.push(command);window.dispatchEvent(new CustomEvent('visigoth-command',{detail:command}));}
function dims(){const d=doc.data.data;return doc.data.view==='run'?[d.map.width,d.map.depth]:[d.map_width,d.map_depth];}
function scale(){const [w,h]=dims();return Math.min(canvas.clientWidth/w,canvas.clientHeight/h);}
function xy(p){const s=scale(),[w,h]=dims();return[(canvas.clientWidth-w*s)/2+p[0]*s,(canvas.clientHeight-h*s)/2+p[1]*s];}
function current(){return doc.data.view==='run'?doc.data.data.frames[frame]:null} function currentTick(){return current()?.tick||0}
function color(side,a=1){return side===0?`rgba(77,166,255,${a})`:side===1?`rgba(255,91,91,${a})`:`rgba(215,215,215,${a})`;}
function terrain(map){const s=scale(),cw=map.cell_size*s;for(let r=0;r<map.rows;r++)for(let c=0;c<map.cols;c++){let n=r*map.cols+c,h=map.heights[n]||0,water=(map.water_depth||[])[n]||0;ctx.fillStyle=water>0?`rgb(25,${65+Math.min(90,water)},120)`:`rgb(${45+h*.25},${53+h*.3},${45+h*.2})`;const p=xy([c*map.cell_size,r*map.cell_size]);ctx.fillRect(p[0],p[1],cw+.5,cw+.5);if(map.passable[n]===false){ctx.fillStyle='#0007';ctx.fillRect(p[0],p[1],cw+.5,cw+.5);}}for(const r of map.resources||[]){const p=xy(r.position),geo=r.kind==='geothermal';ctx.strokeStyle=geo?'#ff9d40':'#c7f0ff';ctx.fillStyle=geo?'#ff9d4044':'#c7f0ff44';ctx.lineWidth=2;ctx.beginPath();ctx.arc(p[0],p[1],Math.max(4,r.radius*s),0,Math.PI*2);ctx.fill();ctx.stroke();}}
function symbol(u){const p=xy(u.position),size=Math.max(6,Math.min(18,u.radius*scale())),key=u.icon_key.toLowerCase(),style=doc.icons.icons[key]||{},im=images[key];ctx.save();ctx.translate(...p);ctx.globalAlpha=u.alive?1:.25;if(im&&im.complete){ctx.drawImage(im,-size,-size,size*2,size*2);}else{ctx.fillStyle=color(u.side);ctx.strokeStyle='#111';ctx.lineWidth=2;ctx.beginPath();const shape=style.fallback||['circle','triangle','square','diamond'][[...key].reduce((h,c)=>(h*16777619^c.charCodeAt(0))>>>0,0)&3];if(shape==='triangle'){ctx.moveTo(0,-size);ctx.lineTo(size,size);ctx.lineTo(-size,size);ctx.closePath();}else if(shape==='square')ctx.rect(-size,-size,size*2,size*2);else if(shape==='diamond'){ctx.moveTo(0,-size);ctx.lineTo(size,0);ctx.lineTo(0,size);ctx.lineTo(-size,0);ctx.closePath();}else ctx.arc(0,0,size,0,Math.PI*2);ctx.fill();ctx.stroke();}ctx.rotate(u.heading);ctx.strokeStyle='#fff';ctx.beginPath();ctx.moveTo(0,0);ctx.lineTo(size*1.4,0);ctx.stroke();if(selected.includes(u.id)){ctx.strokeStyle='#fff';ctx.strokeRect(-size-3,-size-3,size*2+6,size*2+6);}ctx.restore();}
function drawRun(run){terrain(run.map);const f=run.frames[frame];if(!f)return;for(const a of f.aim_attempts){const x=xy(a.origin),y=xy(a.aim_point);ctx.strokeStyle=a.state==='fired'?'#ffd95a':a.state.startsWith('blocked')?'#ff55aa':'#ffffff88';ctx.setLineDash(a.state==='fired'?[]:[4,4]);ctx.beginPath();ctx.moveTo(...x);ctx.lineTo(...y);ctx.stroke();}ctx.setLineDash([]);for(const u of f.units)symbol(u);document.querySelector('#status').textContent=`${f.time.toFixed(2)}s · tick ${f.tick} · ${f.units.filter(u=>u.alive).length} units`;}
function heat(values,max,cols,rows,w,h,colorValue){if(!max)return;for(let r=0;r<rows;r++)for(let c=0;c<cols;c++){const v=values[r*cols+c]/max;if(!v)continue;ctx.fillStyle=colorValue(v);const a=xy([c*w/cols,r*h/rows]),b=xy([(c+1)*w/cols,(r+1)*h/rows]);ctx.fillRect(a[0],a[1],b[0]-a[0]+.5,b[1]-a[1]+.5);}}
function drawAggregate(a){ctx.fillStyle='#263026';const p=xy([0,0]),q=xy([a.map_width,a.map_depth]);ctx.fillRect(p[0],p[1],q[0]-p[0],q[1]-p[1]);if(layer.value==='occupancy')for(const l of a.occupancy)heat(l.values,l.maximum,a.cols,a.rows,a.map_width,a.map_depth,v=>color(l.side,v*.8));else{const l=a[layer.value];heat(l.values,l.maximum,a.cols,a.rows,a.map_width,a.map_depth,v=>layer.value==='deaths'?`rgba(255,40,40,${v})`:`rgba(255,220,50,${v})`);}document.querySelector('#status').textContent=`${a.run_count} runs`;}
function draw(){ctx.clearRect(0,0,canvas.clientWidth,canvas.clientHeight);doc.data.view==='run'?drawRun(doc.data.data):drawAggregate(doc.data.data);}
function loop(t){if(running&&doc.data.view==='run'&&doc.data.data.frames.length&&t-last>100){frame=(frame+1)%doc.data.data.frames.length;slider.value=frame;last=t;draw();}requestAnimationFrame(loop);}play.onclick=()=>{running=!running;play.textContent=running?'Pause':'Play'};slider.oninput=()=>{frame=+slider.value;draw()};layer.onchange=draw;
if(doc.data.view==='run'){slider.max=Math.max(0,doc.data.data.frames.length-1);layer.hidden=true;}else{slider.hidden=true;play.hidden=true;}
canvas.onclick=e=>{if(doc.human_side==null||doc.data.view!=='run')return;const f=current(),rect=canvas.getBoundingClientRect(),m=[e.clientX-rect.left,e.clientY-rect.top];let best=null,bd=24;for(const u of f.units){const p=xy(u.position),d=Math.hypot(p[0]-m[0],p[1]-m[1]);if(d<bd){best=u;bd=d;}}selected=best&&best.side===doc.human_side?[best.id]:[];draw();};
canvas.oncontextmenu=e=>{e.preventDefault();if(doc.human_side==null||!selected.length||doc.data.view!=='run')return;const rect=canvas.getBoundingClientRect(),s=scale(),[w,h]=dims(),ox=(canvas.clientWidth-w*s)/2,oz=(canvas.clientHeight-h*s)/2,pos=[(e.clientX-rect.left-ox)/s,(e.clientY-rect.top-oz)/s],enemy=current().units.find(u=>u.side!==doc.human_side&&Math.hypot(u.position[0]-pos[0],u.position[1]-pos[1])*s<20);emit(enemy?{type:'attack_unit',target:enemy.id,queued:e.shiftKey}:{type:'move',position:pos,queued:e.shiftKey});};
window.onkeydown=e=>{if(doc.human_side==null)return;if(e.code==='Space'){e.preventDefault();running=!running;emit({type:'set_paused',paused:!running},[]);}if(e.code==='Period')emit({type:'step',ticks:1},[]);};
new ResizeObserver(resize).observe(canvas);resize();requestAnimationFrame(loop);
</script></body></html>"##;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trace::{TRACE_FORMAT_VERSION, TraceMap};

    fn empty_run() -> RunTrace {
        RunTrace {
            format_version: TRACE_FORMAT_VERSION,
            seed: 1,
            tick_hz: 30.0,
            map: TraceMap {
                width: 10.0,
                depth: 10.0,
                cols: 1,
                rows: 1,
                cell_size: 10.0,
                heights: vec![0.0],
                water_depth: vec![0.0],
                passable: vec![true],
                resources: vec![],
            },
            frames: vec![],
            outcome: None,
        }
    }

    #[test]
    fn html_is_standalone_and_escapes_script_terminators() {
        let document = ViewerDocument {
            title: "trace </script>".into(),
            data: ViewerData::Run(empty_run()),
            icons: IconCatalog::default(),
            human_side: None,
        };
        let html = document.to_html().unwrap();
        assert!(html.contains("<canvas id=\"view\"></canvas>"));
        assert!(html.contains("visigoth-command"));
        assert!(!html.contains("trace </script>"));
    }

    #[test]
    fn shared_viewer_asset_is_stable_and_dependency_free() {
        let html = crate::viewer_protocol::SHARED_VIEWER_HTML;
        assert!(html.contains("VISIGOTH_SHARED_VIEWER"));
        assert!(html.contains("/viewer-config.json"));
        assert!(html.contains("visigoth.viewer_frame"));
        assert!(html.contains("data-review-action=\"accept\""));
        assert!(!html.contains("https://"));
    }

    #[test]
    fn human_commands_are_tick_ordered_and_deferred() {
        let mut inbox = HumanCommandInbox::default();
        for (sequence, tick) in [(2, 8), (1, 4), (0, 4)] {
            inbox.push(HumanCommand {
                sequence,
                apply_at_tick: tick,
                player: 0,
                unit_ids: vec![1],
                action: HumanAction::Stop,
            });
        }
        let ready = inbox.drain_ready(4);
        assert_eq!(ready.iter().map(|c| c.sequence).collect::<Vec<_>>(), [0, 1]);
        assert!(!inbox.is_empty());
        assert_eq!(inbox.drain_ready(8)[0].sequence, 2);
    }

    #[test]
    fn icon_fallback_is_stable() {
        let catalog = IconCatalog::default();
        assert_eq!(catalog.resolve("ArmBot"), catalog.resolve(" armbot "));
        assert!(catalog.resolve("ArmBot").uri.is_none());
        assert_eq!(base64(b"Man"), "TWFu");
    }
}
