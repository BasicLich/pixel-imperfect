#![feature(clamp)]

use fnv::{FnvHashMap as HashMap, FnvHashSet as HashSet};
use image::GenericImageView;
use indexmap::IndexSet;

use quicksilver::{
    geom::{Rectangle, Vector},
    graphics::{Color, Graphics, Image, PixelFormat},
    input::{Event, GamepadAxis, GamepadButton, Key},
    run, Input, Result, Settings, Timer, Window,
};

const SPRITES: &[u8] = include_bytes!("../static/monochrome_transparent_packed.png");
const SPRITES_WIDTH: usize = 768;
const SPRITES_HEIGHT: usize = 352;
const SPRITE_WIDTH: usize = 16;
const PIXEL_CHUNK: u32 = 4;
const MAX_SCALE: usize = 180;
const SCALE_CHANGE_TIMEOUT: f32 = 1.0;

const FOREGROUND_COLOR: Color = Color {
    r: 100.0 / 255.0,
    g: 200.0 / 255.0,
    b: 100.0 / 255.0,
    a: 1.0,
};
const BACKGROUND_COLOR: Color = Color {
    r: 50.0 / 255.0,
    g: 50.0 / 255.0,
    b: 100.0 / 255.0,
    a: 1.0,
};
const TERRAIN_COLOR: Color = Color {
    r: 50.0 / 255.0,
    g: 100.0 / 255.0,
    b: 50.0 / 255.0,
    a: 1.0,
};

fn main() {
    run(
        Settings {
            title: "Pixel Game!",
            size: Vector::new(1920.0, 1080.0),
            fullscreen: true,
            ..Settings::default()
        },
        app,
    );
}

fn extract_sprite(
    src: &image::DynamicImage,
    x: usize,
    y: usize,
) -> [bool; SPRITE_WIDTH * SPRITE_WIDTH] {
    let mut pixels = vec![0; SPRITE_WIDTH * SPRITE_WIDTH * 4];
    let mut collider = [false; SPRITE_WIDTH * SPRITE_WIDTH];
    let x = x * SPRITE_WIDTH;
    let y = y * SPRITE_WIDTH;
    for dx in 0..SPRITE_WIDTH {
        for dy in 0..SPRITE_WIDTH {
            let i = dx * 4 + dy * 4 * SPRITE_WIDTH;
            let p = src.get_pixel((x + dx) as u32, (y + dy) as u32);
            pixels[i] = p.0[0];
            pixels[i + 1] = p.0[1];
            pixels[i + 2] = p.0[2];
            pixels[i + 3] = p.0[3];
            if p.0[3] > 0 {
                collider[dx + dy * SPRITE_WIDTH] = true;
            }
        }
    }
    collider
}

struct Sprite {
    is_player: bool,
    collider: [bool; SPRITE_WIDTH * SPRITE_WIDTH],
    loc: Vector,
    x_scale: u32,
    y_scale: u32,
    velocity: Vector,
    ground_contact: bool,
    jumping: bool,
    vy_slop: f32,
    color: Color,
    potion_timer: Option<f32>,
    pending_potions: Vec<PotionType>,
    sleep_timer: f32,
    gravity: bool,
}

impl Sprite {
    fn new(
        src: &image::DynamicImage,
        x: usize,
        y: usize,
        xx: f32,
        yy: f32,
        x_scale: u32,
        y_scale: u32,
        color: Color,
    ) -> Self {
        let collider = extract_sprite(src, x, y);
        Sprite::from_collider(collider, xx, yy, x_scale, y_scale, color)
    }

    fn from_collider(
        collider: [bool; SPRITE_WIDTH * SPRITE_WIDTH],
        xx: f32,
        yy: f32,
        x_scale: u32,
        y_scale: u32,
        color: Color,
    ) -> Self {
        Self {
            is_player: false,
            collider,
            loc: Vector::new(xx as f32, yy as f32),
            x_scale,
            y_scale,
            velocity: Vector::new(0.0, 0.0),
            ground_contact: false,
            jumping: false,
            vy_slop: 0.0,
            color,
            potion_timer: None,
            pending_potions: Vec::new(),
            sleep_timer: 0.0,
            gravity: true,
        }
    }

    fn maybe_flip(mut self, flip: bool) -> Self {
        if flip {
            let mut collider = [false; SPRITE_WIDTH * SPRITE_WIDTH];
            for x in 0..SPRITE_WIDTH {
                for y in 0..SPRITE_WIDTH {
                    let src_i = x + y * SPRITE_WIDTH;
                    let dst_i = (SPRITE_WIDTH - x - 1) + y * SPRITE_WIDTH;
                    collider[dst_i] = self.collider[src_i];
                }
            }
            self.collider = collider;
        }

        self
    }

    fn quarter(self) -> Vec<Self> {
        let Self {
            is_player,
            collider,
            loc,
            x_scale,
            y_scale,
            velocity,
            ground_contact,
            jumping,
            vy_slop,
            color,
            potion_timer,
            pending_potions,
            sleep_timer,
            gravity,
        } = self;
        let new_x_scale = x_scale / 2;
        let new_y_scale = y_scale / 2;
        [
            (0, 0),
            (SPRITE_WIDTH / 2 - 1, 0),
            (0, SPRITE_WIDTH / 2 - 1),
            (SPRITE_WIDTH / 2 - 1, SPRITE_WIDTH / 2 - 1),
        ]
        .iter()
        .map(|(dx, dy)| {
            let mut new_collider = [false; SPRITE_WIDTH * SPRITE_WIDTH];
            for x in 0..SPRITE_WIDTH {
                for y in 0..SPRITE_WIDTH {
                    let src_i = x / 2 + dx + (y / 2 + dy) * SPRITE_WIDTH;
                    let dst_i = x + y * SPRITE_WIDTH;
                    new_collider[dst_i] = collider[src_i];
                }
            }
            Self {
                is_player,
                collider: new_collider,
                loc: Vector::new(
                    loc.x + *dx as f32 * x_scale as f32,
                    loc.y + *dy as f32 * y_scale as f32,
                ),
                x_scale: new_x_scale,
                y_scale: new_y_scale,
                velocity,
                ground_contact,
                jumping,
                vy_slop,
                color,
                potion_timer,
                pending_potions: pending_potions.clone(),
                sleep_timer,
                gravity,
            }
        })
        .collect()
    }

    fn overlap(&self, other: &Sprite) -> bool {
        let a = vek::geom::Rect::new(
            self.loc.x as i32,
            self.loc.y as i32,
            SPRITE_WIDTH as i32 * self.x_scale as i32,
            SPRITE_WIDTH as i32 * self.y_scale as i32,
        );
        let b = vek::geom::Rect::new(
            other.loc.x as i32,
            other.loc.y as i32,
            SPRITE_WIDTH as i32 * other.x_scale as i32,
            SPRITE_WIDTH as i32 * other.y_scale as i32,
        );
        if a.collides_with_rect(b) {
            let c = a.intersection(b);
            for x in c.x..c.x + c.w {
                for y in c.y..c.y + c.h {
                    let (dx, dy) = to_scale(
                        x as i32 - self.loc.x as i32,
                        y as i32 - self.loc.y as i32,
                        self.x_scale,
                        self.y_scale,
                    );
                    let ai = dx as usize + dy as usize * SPRITE_WIDTH;
                    if self.collider[ai] {
                        let (dx, dy) = to_scale(
                            x as i32 - other.loc.x as i32,
                            y as i32 - other.loc.y as i32,
                            other.x_scale,
                            other.y_scale,
                        );
                        let bi = dx as usize + dy as usize * SPRITE_WIDTH;
                        if other.collider[bi] {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    fn image(&self, gfx: &Graphics) -> Image {
        let mut pixels = [0; SPRITE_WIDTH * SPRITE_WIDTH * 4];
        for (i, src) in self.collider.iter().enumerate() {
            if *src {
                pixels[i * 4] = (self.color.r * 255.0).clamp(0.0, 255.0) as u8;
                pixels[i * 4 + 1] = (self.color.g * 255.0).clamp(0.0, 255.0) as u8;
                pixels[i * 4 + 2] = (self.color.b * 255.0).clamp(0.0, 255.0) as u8;
                pixels[i * 4 + 3] = 0xff;
            }
        }
        let mut image = Image::from_raw(
            gfx,
            Some(&pixels),
            SPRITE_WIDTH as u32,
            SPRITE_WIDTH as u32,
            PixelFormat::RGBA,
        )
        .unwrap();
        image
            .set_magnification(golem::TextureFilter::Nearest)
            .unwrap();
        image
    }
}

const LEAF_SIZE: usize = 64;
struct CollisionTree {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    free_pixels: u32,
    children: Option<Vec<CollisionTree>>,
    grid: Option<[bool; LEAF_SIZE * LEAF_SIZE]>,
}

impl CollisionTree {
    fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        let width = ((width as f32 / 4.0).ceil() * 4.0) as u32;
        let height = ((height as f32 / 4.0).ceil() * 4.0) as u32;
        Self {
            x,
            y,
            width,
            height,
            free_pixels: width * height,
            children: None,
            grid: None,
        }
    }

    fn clear(&mut self) {
        self.free_pixels = self.width * self.height;
        self.children.take();
        self.grid.take();
    }

    fn insert(&mut self, x: i32, y: i32) -> std::result::Result<bool, ()> {
        if x < self.x
            || x >= self.x + self.width as i32
            || y < self.y
            || y >= self.y + self.height as i32
        {
            return Err(());
        }
        if self.free_pixels == 0 {
            return Ok(false);
        } else {
            if let Some(children) = &mut self.children {
                for child in children {
                    if x >= child.x
                        && x < child.x + child.width as i32
                        && y >= child.y
                        && y < child.y + child.height as i32
                    {
                        let e = child.insert(x, y);
                        if let Ok(true) = &e {
                            self.free_pixels -= 1;
                        }
                        return e;
                    }
                }
            } else {
                if self.width * self.height > (LEAF_SIZE * LEAF_SIZE) as u32 {
                    self.children = Some(vec![
                        CollisionTree::new(self.x, self.y, self.width / 2, self.height / 2),
                        CollisionTree::new(
                            self.x + self.width as i32 / 2,
                            self.y,
                            self.width / 2,
                            self.height / 2,
                        ),
                        CollisionTree::new(
                            self.x + self.width as i32 / 2,
                            self.y + self.height as i32 / 2,
                            self.width / 2,
                            self.height / 2,
                        ),
                        CollisionTree::new(
                            self.x,
                            self.y + self.height as i32 / 2,
                            self.width / 2,
                            self.width / 2,
                        ),
                    ]);
                    for child in self.children.as_mut().unwrap().iter_mut() {
                        if x >= child.x
                            && x < child.x + child.width as i32
                            && y >= child.y
                            && y < child.y + child.height as i32
                        {
                            let e = child.insert(x, y);
                            if let Ok(true) = &e {
                                self.free_pixels -= 1;
                            }
                            return e;
                        }
                    }
                } else {
                    if self.grid.is_none() {
                        self.grid.replace([false; LEAF_SIZE * LEAF_SIZE]);
                    }
                    let i = ((x - self.x) + (y - self.y) * self.width as i32) as usize;
                    let p = &mut self.grid.as_mut().unwrap()[i];
                    if !*p {
                        *p = true;
                        self.free_pixels -= 1;
                        return Ok(true);
                    } else {
                        return Ok(false);
                    }
                }
            }
        }
        unreachable!();
    }

    fn add_sprite(&mut self, sprite: &Sprite) {
        for x in 0..SPRITE_WIDTH {
            for y in 0..SPRITE_WIDTH {
                let i = x + y * SPRITE_WIDTH;
                if sprite.collider[i] {
                    let rx = x as i32 * sprite.x_scale as i32 + sprite.loc.x as i32;
                    let ry = y as i32 * sprite.y_scale as i32 + sprite.loc.y as i32;
                    if let Ok(x) = self.insert_rect(rx, ry, sprite.x_scale, sprite.y_scale) {
                        if x > 0 {}
                    }
                }
            }
        }
    }

    fn clear_sprite(&mut self, sprite: Sprite) {
        for x in 0..SPRITE_WIDTH {
            for y in 0..SPRITE_WIDTH {
                let i = x + y * SPRITE_WIDTH;
                if sprite.collider[i] {
                    let rx = x as i32 * sprite.x_scale as i32 + sprite.loc.x as i32;
                    let ry = y as i32 * sprite.y_scale as i32 + sprite.loc.y as i32;
                    self.remove_rect(rx, ry, sprite.x_scale, sprite.y_scale);
                }
            }
        }
    }

    fn check_point(&self, x: i32, y: i32) -> bool {
        if x < self.x
            || x >= self.x + self.width as i32
            || y < self.y
            || y >= self.y + self.height as i32
        {
            return false;
        }
        if self.free_pixels == 0 {
            return true;
        }
        if let Some(grid) = &self.grid {
            let x = x - self.x;
            let y = y - self.y;
            let i = (x + y * self.width as i32) as usize;
            return grid[i];
        } else {
            if let Some(children) = &self.children {
                for child in children {
                    if child.check_point(x, y) {
                        return true;
                    }
                }
            }
        }
        false
    }

    fn insert_rect(
        &mut self,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
    ) -> std::result::Result<u32, ()> {
        if x + width as i32 <= self.x
            || x > self.x + self.width as i32
            || y + height as i32 <= self.y
            || y > self.y + self.height as i32
        {
            return Err(());
        }

        if x <= self.x
            && x + width as i32 > self.x + self.width as i32
            && y <= self.y
            && y + height as i32 > self.y + self.height as i32
        {
            let change = self.free_pixels;
            self.free_pixels = 0;
            if self.width * self.height <= (LEAF_SIZE * LEAF_SIZE) as u32 {
                self.grid.replace([true; LEAF_SIZE * LEAF_SIZE]);
            } else {
                if self.children.is_none() {
                    self.children = Some(vec![
                        CollisionTree::new(self.x, self.y, self.width / 2, self.height / 2),
                        CollisionTree::new(
                            self.x + self.width as i32 / 2,
                            self.y,
                            self.width / 2,
                            self.height / 2,
                        ),
                        CollisionTree::new(
                            self.x + self.width as i32 / 2,
                            self.y + self.height as i32 / 2,
                            self.width / 2,
                            self.height / 2,
                        ),
                        CollisionTree::new(
                            self.x,
                            self.y + self.height as i32 / 2,
                            self.width / 2,
                            self.width / 2,
                        ),
                    ]);
                }
                if let Some(children) = self.children.as_mut() {
                    for child in children {
                        child.insert_rect(x, y, width, height);
                    }
                }
            }
            return Ok(change);
        }

        if self.width * self.height <= (LEAF_SIZE * LEAF_SIZE) as u32 {
            if self.grid.is_none() {
                self.grid.replace([false; LEAF_SIZE * LEAF_SIZE]);
            }
            if let Some(grid) = &mut self.grid {
                let mut inserted = 0;
                for x in self.x.max(x)..(self.x + self.width as i32).min(x + width as i32) {
                    for y in self.y.max(y)..(self.y + self.height as i32).min(y + height as i32) {
                        let x = x - self.x;
                        let y = y - self.y;
                        let i = (x + y * self.width as i32) as usize;
                        if !grid[i] {
                            self.free_pixels -= 1;
                            inserted += 1;
                            grid[i] = true;
                        }
                    }
                }
                return Ok(inserted);
            }
        } else {
            let mut inserted = 0;
            if self.children.is_none() {
                self.children = Some(vec![
                    CollisionTree::new(self.x, self.y, self.width / 2, self.height / 2),
                    CollisionTree::new(
                        self.x + self.width as i32 / 2,
                        self.y,
                        self.width / 2,
                        self.height / 2,
                    ),
                    CollisionTree::new(
                        self.x + self.width as i32 / 2,
                        self.y + self.height as i32 / 2,
                        self.width / 2,
                        self.height / 2,
                    ),
                    CollisionTree::new(
                        self.x,
                        self.y + self.height as i32 / 2,
                        self.width / 2,
                        self.width / 2,
                    ),
                ]);
            }
            if let Some(children) = self.children.as_mut() {
                for child in children {
                    if let Ok(change) = child.insert_rect(x, y, width, height) {
                        inserted += change;
                    }
                }
            }
            self.free_pixels -= inserted;
            return Ok(inserted);
        }
        unreachable!();
    }

    fn remove_rect(&mut self, x: i32, y: i32, width: u32, height: u32) -> (bool, u32) {
        if x + width as i32 <= self.x
            || x > self.x + self.width as i32
            || y + height as i32 <= self.y
            || y > self.y + self.height as i32
        {
            return (false, 0);
        }
        if x <= self.x
            && x + width as i32 > self.x + self.width as i32
            && y <= self.y
            && y + height as i32 > self.y + self.height as i32
        {
            self.children.take();
            self.grid.take();
            return (true, self.width * self.height - self.free_pixels);
        }
        if let Some(grid) = &mut self.grid {
            let mut removed = 0;
            for x in self.x.max(x)..(self.x + self.width as i32).min(x + width as i32) {
                for y in self.y.max(y)..(self.y + self.height as i32).min(y + height as i32) {
                    let x = x - self.x;
                    let y = y - self.y;
                    let i = (x + y * self.width as i32) as usize;
                    if grid[i] {
                        self.free_pixels += 1;
                        removed += 1;
                        grid[i] = false;
                    }
                }
            }
            if self.free_pixels == self.width * self.height {
                return (true, removed);
            }
            return (false, removed);
        } else {
            let mut keep = false;
            let mut removed = 0;
            if let Some(children) = &mut self.children {
                for child in children {
                    let (empty, child_removed) = child.remove_rect(x, y, width, height);
                    removed += child_removed;
                    if !empty {
                        keep = true;
                    } else {
                    }
                }
            }
            if !keep {
                self.children.take();
            }
            self.free_pixels += removed;
            return (!keep, removed);
        }
        (false, 0)
    }

    fn check_rect(&self, x: i32, y: i32, width: u32, height: u32) -> bool {
        if x + width as i32 <= self.x
            || x > self.x + self.width as i32
            || y + height as i32 <= self.y
            || y > self.y + self.height as i32
        {
            return false;
        }
        if self.free_pixels == 0 {
            return true;
        }
        if self.free_pixels == self.width * self.height {
            return false;
        }
        if x <= self.x
            && x + width as i32 > self.x + self.width as i32
            && y <= self.y
            && y + height as i32 > self.y + self.height as i32
        {
            if self.free_pixels < self.width * self.height {
                return true;
            }
        }
        if let Some(grid) = &self.grid {
            for x in self.x.max(x)..(self.x + self.width as i32).min(x + width as i32) {
                for y in self.y.max(y)..(self.y + self.height as i32).min(y + height as i32) {
                    let x = x - self.x;
                    let y = y - self.y;
                    let i = (x + y * self.width as i32) as usize;
                    if grid[i] {
                        return true;
                    }
                }
            }
        } else {
            if let Some(children) = &self.children {
                for child in children {
                    if child.check_rect(x, y, width, height) {
                        return true;
                    }
                }
            }
        }
        false
    }
}

const TILE_SIZE: u32 = 256;

#[derive(Copy, Clone)]
enum PotionType {
    Relative(i32, i32),
    Absolute(Option<i32>, Option<i32>),
}
struct Scene {
    sprites: HashMap<usize, Sprite>,
    sprite_cache: HashMap<usize, Image>,
    potions: Vec<(usize, PotionType, bool)>,
    characters: Vec<usize>,
    particles: Vec<usize>,
    collectables: Vec<usize>,
    collected: HashMap<usize, Sprite>,
    collision_map: CollisionTree,
    rubble_map: CollisionTree,
    next_id: usize,
    foreground_map: CollisionTree,
    background_map: CollisionTree,
    tile_cache: HashMap<
        (i32, i32),
        (
            (Option<Vec<u8>>, Option<Image>),
            (Option<Vec<u8>>, Option<Image>),
            (Option<Vec<u8>>, Option<Image>),
        ),
    >,
    tile_queue: IndexSet<(u32, i32, i32)>,
    score: u32,
    final_potion_triggered: bool,
    end_sequence_triggered: bool,
    done: bool,
}

fn to_scale(x: i32, y: i32, x_scale: u32, y_scale: u32) -> (i32, i32) {
    let x = x / x_scale as i32;
    let y = y / y_scale as i32;
    (x, y)
}

fn from_scale(x: i32, y: i32, x_scale: u32, y_scale: u32) -> (i32, i32) {
    let x = x * x_scale as i32;
    let y = y * y_scale as i32;
    (x, y)
}

impl Scene {
    fn new() -> Self {
        let world_min = -10000;
        let world_width = 40000;
        let mut tile_cache = HashMap::default();
        for x in world_min / TILE_SIZE as i32..(world_min + world_width) / TILE_SIZE as i32 {
            for y in world_min / TILE_SIZE as i32..(world_min + world_width) / TILE_SIZE as i32 {
                tile_cache.insert((x, y), ((None, None), (None, None), (None, None)));
            }
        }
        Self {
            sprites: HashMap::default(),
            sprite_cache: HashMap::default(),
            potions: vec![],
            characters: vec![],
            particles: vec![],
            collectables: vec![],
            collected: Default::default(),
            collision_map: CollisionTree::new(
                world_min,
                world_min,
                world_width as u32,
                world_width as u32,
            ),
            rubble_map: CollisionTree::new(
                world_min,
                world_min,
                world_width as u32,
                world_width as u32,
            ),
            next_id: 0,
            tile_cache,
            foreground_map: CollisionTree::new(
                world_min,
                world_min,
                world_width as u32,
                world_width as u32,
            ),
            background_map: CollisionTree::new(
                world_min,
                world_min,
                world_width as u32,
                world_width as u32,
            ),
            tile_queue: IndexSet::default(),
            score: 0,
            final_potion_triggered: false,
            end_sequence_triggered: false,
            done: false,
        }
    }

    fn add_sprite(&mut self, sprite: Sprite) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        self.sprites.insert(id, sprite);
        id
    }

    fn add_collectable(&mut self, sprite: Sprite) -> usize {
        let id = self.add_sprite(sprite);
        self.collectables.push(id);
        id
    }

    fn add_potion(&mut self, sprite: Sprite, potion_type: PotionType, start_end: bool) -> usize {
        let id = self.add_sprite(sprite);
        self.potions.push((id, potion_type, start_end));
        id
    }

    fn add_particle(&mut self, sprite: Sprite) -> usize {
        let id = self.add_sprite(sprite);
        self.particles.push(id);
        id
    }

    fn add_character(&mut self, sprite: Sprite) -> usize {
        let id = self.add_sprite(sprite);
        self.characters.push(id);
        id
    }

    fn add_terrain(&mut self, sprite: &Sprite) {
        self.collision_map.add_sprite(sprite);
        for x in
            sprite.loc.x as i32..sprite.loc.x as i32 + SPRITE_WIDTH as i32 * sprite.x_scale as i32
        {
            for y in sprite.loc.y as i32
                ..sprite.loc.y as i32 + SPRITE_WIDTH as i32 * sprite.y_scale as i32
            {
                self.tile_queue
                    .insert((1, x / TILE_SIZE as i32, y / TILE_SIZE as i32));
                self.tile_cache
                    .entry((x / TILE_SIZE as i32, y / TILE_SIZE as i32))
                    .or_default()
                    .1 = (None, None);
            }
        }
    }

    fn clear_terrain(&mut self, sprite: Sprite) {
        for x in
            sprite.loc.x as i32..sprite.loc.x as i32 + SPRITE_WIDTH as i32 * sprite.x_scale as i32
        {
            for y in sprite.loc.y as i32
                ..sprite.loc.y as i32 + SPRITE_WIDTH as i32 * sprite.y_scale as i32
            {
                self.tile_cache
                    .remove(&(x / TILE_SIZE as i32, y / TILE_SIZE as i32));
            }
        }
        self.collision_map.clear_sprite(sprite);
    }

    fn add_foreground(&mut self, sprite: &Sprite) {
        self.foreground_map.add_sprite(sprite);
        for x in
            sprite.loc.x as i32..sprite.loc.x as i32 + SPRITE_WIDTH as i32 * sprite.x_scale as i32
        {
            for y in sprite.loc.y as i32
                ..sprite.loc.y as i32 + SPRITE_WIDTH as i32 * sprite.y_scale as i32
            {
                self.tile_cache
                    .entry((x / TILE_SIZE as i32, y / TILE_SIZE as i32))
                    .or_default()
                    .2 = (None, None);
                self.tile_queue
                    .insert((2, x / TILE_SIZE as i32, y / TILE_SIZE as i32));
            }
        }
    }

    fn add_background(&mut self, sprite: &Sprite) {
        self.background_map.add_sprite(sprite);
        for x in
            sprite.loc.x as i32..sprite.loc.x as i32 + SPRITE_WIDTH as i32 * sprite.x_scale as i32
        {
            for y in sprite.loc.y as i32
                ..sprite.loc.y as i32 + SPRITE_WIDTH as i32 * sprite.y_scale as i32
            {
                self.tile_cache
                    .entry((x / TILE_SIZE as i32, y / TILE_SIZE as i32))
                    .or_default()
                    .0 = (None, None);
                self.tile_queue
                    .insert((0, x / TILE_SIZE as i32, y / TILE_SIZE as i32));
            }
        }
    }

    fn step_physics(&mut self, camera: Vector, camera_scale: f32, fps: f32) {
        let mut new_sprites = vec![];
        for sprite in self.sprites.values_mut() {
            if camera.distance(sprite.loc) > 1920.0 * camera_scale {
                continue;
            }

            if sprite.gravity {
                sprite.velocity.y += 3.4 / fps;
            }
            let mut blocked_x = false;
            let mut blocked_y = false;
            let mut blocked_by_ground = false;
            let mut in_rubble = false;
            let falling = sprite.velocity.y > 0.0;
            for (mut vx, mut vy) in vec![
                (0, (sprite.velocity.y * sprite.y_scale as f32) as i32),
                ((sprite.velocity.x * sprite.x_scale as f32) as i32, 0),
            ] {
                {
                    let mut loc_x = sprite.loc.x;
                    let mut loc_y = sprite.loc.y;

                    let step_x = (sprite.x_scale as f32 / 8.0)
                        .min(1.0)
                        .min(sprite.velocity.x.abs())
                        .max(1.0)
                        .copysign(sprite.velocity.x);
                    let step_y = (sprite.y_scale as f32 / 8.0)
                        .min(1.0)
                        .min(sprite.velocity.y.abs())
                        .max(1.0)
                        .copysign(sprite.velocity.y);

                    'outer: while vy.abs() >= 1 || vx.abs() >= 1 {
                        if vy.abs() >= 1 {
                            loc_y += step_y;
                        } else {
                            loc_x += step_x;
                        }
                        for dx in 0..SPRITE_WIDTH {
                            for dy in 0..SPRITE_WIDTH {
                                let i = dx + dy * SPRITE_WIDTH;
                                if sprite.collider[i] {
                                    let x = loc_x as i32 + dx as i32 * sprite.x_scale as i32;
                                    let y = loc_y as i32 + dy as i32 * sprite.y_scale as i32;
                                    if self.rubble_map.check_rect(
                                        x,
                                        y,
                                        sprite.x_scale,
                                        sprite.y_scale,
                                    ) {
                                        in_rubble = true;
                                    } else if self.collision_map.check_rect(
                                        x,
                                        y,
                                        sprite.x_scale,
                                        sprite.y_scale,
                                    ) {
                                        if vx.abs() >= 1 {
                                            blocked_x = true;
                                        } else {
                                            blocked_y = true;
                                        }
                                        break 'outer;
                                    }
                                }
                            }
                        }
                        if vy.abs() >= 1 {
                            vy -= step_y as i32;
                        } else {
                            vx -= step_x as i32;
                        }
                        sprite.loc.x = loc_x;
                        sprite.loc.y = loc_y;
                    }
                }
            }
            if sprite.is_player && !in_rubble {
                self.rubble_map.clear();
            }
            if !blocked_y {
                if sprite.velocity.y.abs() >= 1.0 {
                    sprite.ground_contact = false;
                }
            } else {
                if falling {
                    sprite.ground_contact = true;
                    sprite.jumping = false;
                }
                sprite.velocity.y = 0.0;
            }
            if sprite.ground_contact {
                if sprite.velocity.x >= 0.0 {
                    sprite.velocity.x = (sprite.velocity.x - 1.0 / fps).max(0.0);
                } else {
                    sprite.velocity.x = (sprite.velocity.x + 1.0 / fps).min(0.0);
                }
            }
            if sprite.velocity.x.abs() > 1.0 || sprite.velocity.y.abs() > 1.0 {
                sprite.sleep_timer = 0.0;
            } else {
                sprite.sleep_timer += 1.0 / fps;
            }
            if sprite.is_player {
                //&& sprite.y_scale < 80 {
                // Collision resolution
                let mut x_dir = 0;
                let mut y_dir = 0;
                for dx in 0..SPRITE_WIDTH {
                    for dy in 0..SPRITE_WIDTH {
                        let i = dx + dy * SPRITE_WIDTH;
                        if sprite.collider[i] {
                            let x = sprite.loc.x as i32 + dx as i32 * sprite.x_scale as i32;
                            let y = sprite.loc.y as i32 + dy as i32 * sprite.y_scale as i32;
                            if self
                                .rubble_map
                                .check_rect(x, y, sprite.x_scale, sprite.y_scale)
                            {
                            } else if self.collision_map.check_rect(
                                x,
                                y,
                                sprite.x_scale,
                                sprite.y_scale,
                            ) {
                                if dx <= SPRITE_WIDTH / 2 {
                                    x_dir += 1;
                                } else {
                                    x_dir -= 1;
                                }
                                if dy <= SPRITE_WIDTH / 2 {
                                    y_dir += 1;
                                } else {
                                    y_dir -= 1;
                                }
                            }
                        }
                    }
                }
                sprite.loc.x += (x_dir.max(-1).min(1) * sprite.x_scale as i32) as f32;
                sprite.loc.y += (y_dir.max(-1).min(1) * sprite.y_scale as i32) as f32;
            }
        }

        let mut to_remove = HashSet::default();
        for particle_id in &self.particles {
            let sprite = &self.sprites[particle_id];
            if sprite.loc.y > 30000.0 {
                to_remove.insert(*particle_id);
                self.sprite_cache.remove(particle_id);
                continue;
            }
            if sprite.ground_contact && sprite.sleep_timer > 0.5 {
                to_remove.insert(*particle_id);
                for x in 0..SPRITE_WIDTH {
                    for y in 0..SPRITE_WIDTH {
                        let i = x + y * SPRITE_WIDTH;
                        if sprite.collider[i as usize] {
                            for dx in 0..sprite.x_scale {
                                for dy in 0..sprite.y_scale {
                                    let x = sprite.loc.x as i32
                                        + x as i32 * sprite.x_scale as i32
                                        + dx as i32;
                                    let y = sprite.loc.y as i32
                                        + y as i32 * sprite.y_scale as i32
                                        + dy as i32;
                                    self.collision_map.insert(x, y);
                                    self.rubble_map.insert(x, y);
                                    self.tile_cache
                                        .entry((x / TILE_SIZE as i32, y / TILE_SIZE as i32))
                                        .or_default()
                                        .1 = (None, None);
                                    self.tile_queue.insert((
                                        1,
                                        x / TILE_SIZE as i32,
                                        y / TILE_SIZE as i32,
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }
        self.particles.retain(|pid| !to_remove.contains(pid));
        self.sprites.retain(|pid, _| !to_remove.contains(pid));

        let mut drinkers = vec![];
        let mut consumed = HashSet::default();
        let mut collected = HashSet::default();
        let mut start_end = false;
        for character_id in &self.characters {
            let character = &self.sprites[character_id];
            for (potion_id, potion_type, end) in &self.potions {
                if consumed.contains(potion_id) {
                    continue;
                }
                let potion = &self.sprites[potion_id];
                if character.overlap(potion) {
                    consumed.insert(*potion_id);
                    drinkers.push((*character_id, *potion_type));
                    start_end |= *end;
                }
            }
            for collectable_id in &self.collectables {
                if collected.contains(collectable_id) {
                    continue;
                }
                let collectable = &self.sprites[collectable_id];
                if character.overlap(collectable) {
                    if collectable.x_scale > 30 {
                        self.done = true;
                    }
                    collected.insert(*collectable_id);
                }
            }
        }
        for potion_id in consumed {
            self.potions.retain(|(id, _, _)| *id != potion_id);
            self.sprites.remove(&potion_id);
            self.sprite_cache.remove(&potion_id);
        }
        if start_end {
            self.end_sequence_triggered = true;
            self.potions
                .iter_mut()
                .for_each(|(_, pt, _)| *pt = PotionType::Relative(10, 10));
        }
        for collectable_id in collected {
            self.collectables.retain(|id| *id != collectable_id);
            self.collected.insert(
                collectable_id,
                self.sprites.remove(&collectable_id).unwrap(),
            );
            self.score += 1;
        }
        for (sprite_id, potion_type) in drinkers {
            let sprite = self.sprites.get_mut(&sprite_id).unwrap();
            let timer = sprite.potion_timer.get_or_insert(SCALE_CHANGE_TIMEOUT);
            if *timer <= 0.0 {
                *timer = SCALE_CHANGE_TIMEOUT;
            }
            sprite.pending_potions.push(potion_type);
        }

        for character_id in self.characters.clone() {
            let sprite = self.sprites.get_mut(&character_id).unwrap();
            if let Some(time) = sprite.potion_timer.as_mut() {
                *time -= 1.0 / fps;
                if *time > 0.0 && !self.final_potion_triggered {
                    continue;
                }
                if *time < -1.0 {
                    sprite.potion_timer.take();
                }
                let mut x_scale = sprite.x_scale as i32;
                let mut y_scale = sprite.y_scale as i32;
                for potion in sprite.pending_potions.drain(..) {
                    match potion {
                        PotionType::Relative(dx, dy) => {
                            x_scale += dx;
                            y_scale += dy;
                        }
                        PotionType::Absolute(x, y) => {
                            if let Some(x) = x {
                                x_scale = x;
                            }
                            if let Some(y) = y {
                                y_scale = y;
                            }
                        }
                    }
                }
                if self.end_sequence_triggered {
                    self.final_potion_triggered = true;
                }

                let x_delta;
                let y_delta;
                if self.final_potion_triggered {
                    x_delta = 20;
                    y_delta = 20;
                } else {
                    x_delta = x_scale.max(0).min(MAX_SCALE as i32) - sprite.x_scale as i32;
                    y_delta = y_scale.max(0).min(MAX_SCALE as i32) - sprite.y_scale as i32;
                }
                if x_delta == 0 && y_delta == 0 {
                    continue;
                }
                let initial_width = SPRITE_WIDTH as u32 * sprite.x_scale;
                let initial_height = SPRITE_WIDTH as u32 * sprite.y_scale;
                sprite.x_scale = (sprite.x_scale as i32 + x_delta)
                    .max(1)
                    .min(MAX_SCALE as i32) as u32;
                sprite.y_scale = (sprite.y_scale as i32 + y_delta)
                    .max(1)
                    .min(MAX_SCALE as i32) as u32;
                sprite.loc.x -=
                    (SPRITE_WIDTH as f32 * sprite.x_scale as f32 - initial_width as f32) / 2.0;
                sprite.loc.y -= SPRITE_WIDTH as f32 * sprite.y_scale as f32 - initial_height as f32;
                //FIXME: Why is this offset necessary?
                sprite.loc.y -= 8.0;
                if x_delta > 0 || y_delta > 0 {
                    let cx = sprite.loc.x + (SPRITE_WIDTH * sprite.x_scale as usize) as f32 / 2.0;
                    let cy = sprite.loc.y + (SPRITE_WIDTH * sprite.y_scale as usize) as f32 / 2.0;
                    let shape: Vec<_> = if sprite.y_scale < MAX_SCALE as u32 {
                        (0..SPRITE_WIDTH as i32)
                            .flat_map(|x| (-1..SPRITE_WIDTH as i32 - 1).map(move |y| (x, y)))
                            .collect()
                    } else {
                        (-(SPRITE_WIDTH as i32) * 10..SPRITE_WIDTH as i32)
                            .flat_map(|y| (0..SPRITE_WIDTH as i32).map(move |x| (x, y)))
                            .collect()
                    };
                    for (dx, dy) in shape {
                        if true {
                            let x = sprite.loc.x as i32 + dx as i32 * sprite.x_scale as i32;
                            let y = sprite.loc.y as i32 + dy as i32 * sprite.y_scale as i32;
                            if Vector::new(cx, cy).distance(Vector::new(x as f32, y as f32))
                                < SPRITE_WIDTH as f32
                                    * sprite.x_scale.max(sprite.y_scale) as f32
                                    * 0.5
                            {
                                if self
                                    .foreground_map
                                    .remove_rect(x, y, sprite.x_scale, sprite.y_scale)
                                    .1
                                    > 0
                                {
                                    for xx in (sprite.loc.x as i32
                                        + dx as i32 * sprite.x_scale as i32)
                                        / TILE_SIZE as i32
                                        ..(sprite.loc.x as i32
                                            + (dx + 1) as i32 * sprite.x_scale as i32)
                                            / TILE_SIZE as i32
                                    {
                                        for yy in (sprite.loc.y as i32
                                            + dy as i32 * sprite.y_scale as i32)
                                            / TILE_SIZE as i32
                                            ..(sprite.loc.y as i32
                                                + (dy + 1) as i32 * sprite.y_scale as i32)
                                                / TILE_SIZE as i32
                                        {
                                            let cached =
                                                self.tile_cache.entry((xx, yy)).or_default();
                                            cached.2 = (None, None);
                                            self.tile_queue.insert((2, xx, yy));
                                        }
                                    }
                                }
                                if self
                                    .collision_map
                                    .remove_rect(x, y, sprite.x_scale, sprite.y_scale)
                                    .1
                                    > 0
                                {
                                    if new_sprites.len() + self.particles.len() < 300 {
                                        let mut collider = [false; SPRITE_WIDTH * SPRITE_WIDTH];
                                        collider[0] = true;
                                        let mut new_sprite = Sprite::from_collider(
                                            collider,
                                            x as f32,
                                            y as f32,
                                            sprite.x_scale,
                                            sprite.y_scale,
                                            TERRAIN_COLOR,
                                        );
                                        let a = (cy - y as f32).atan2(cx - x as f32);
                                        new_sprite.velocity =
                                            Vector::new(a.cos() * -0.5, a.sin() * -0.5);
                                        new_sprites.push(new_sprite);
                                    }
                                    for xx in (sprite.loc.x as i32
                                        + dx as i32 * sprite.x_scale as i32)
                                        / TILE_SIZE as i32
                                        ..(sprite.loc.x as i32
                                            + (dx + 1) as i32 * sprite.x_scale as i32)
                                            / TILE_SIZE as i32
                                            + 1
                                    {
                                        for yy in (sprite.loc.y as i32
                                            + dy as i32 * sprite.y_scale as i32)
                                            / TILE_SIZE as i32
                                            ..(sprite.loc.y as i32
                                                + (dy + 1) as i32 * sprite.y_scale as i32)
                                                / TILE_SIZE as i32
                                                + 1
                                        {
                                            let cached =
                                                self.tile_cache.entry((xx, yy)).or_default();
                                            cached.1 = (None, None);
                                            self.tile_queue.insert((1, xx, yy));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        if !new_sprites.is_empty() {
            //self.rubble_map.clear();
        }
        for sprite in new_sprites {
            self.add_particle(sprite);
        }
    }

    fn draw(&mut self, gfx: &mut Graphics, x: i32, y: i32, width: u32, height: u32, scale: f32) {
        let pwidth = width;
        let pheight = height;
        let x = x - (width as f32 * scale * 0.5) as i32;
        let y = y - (height as f32 * scale * 0.5) as i32;
        let width = (width as f32 * scale) as u32;
        let height = (height as f32 * scale) as u32;

        let mut foreground_tiles = vec![];
        let mut terrain_tiles = vec![];
        let mut background_tiles = vec![];

        for xx in x / TILE_SIZE as i32 - 1..(x + width as i32) / TILE_SIZE as i32 + 1 {
            for yy in y / TILE_SIZE as i32 - 1..(y + height as i32) / TILE_SIZE as i32 + 1 {
                if let Some((background, terrain, foreground)) = self.tile_cache.get_mut(&(xx, yy))
                {
                    let region = Rectangle::new(
                        Vector::new(
                            ((xx * TILE_SIZE as i32 - x) as f32 / scale).floor(),
                            ((yy * TILE_SIZE as i32 - y) as f32 / scale).floor(),
                        ),
                        (Vector::new(
                            (TILE_SIZE as f32 / scale).ceil(),
                            (TILE_SIZE as f32 / scale).ceil(),
                        )),
                    );
                    for ((data, image), ref mut accumulator) in vec![
                        (background, &mut background_tiles),
                        (terrain, &mut terrain_tiles),
                        (foreground, &mut foreground_tiles),
                    ] {
                        if let Some(tile) = image {
                            accumulator.push((region, (xx, yy)));
                        } else if let Some(data) = data {
                            let mut tile = Image::from_raw(
                                gfx,
                                Some(data),
                                TILE_SIZE,
                                TILE_SIZE,
                                PixelFormat::RGBA,
                            )
                            .unwrap();
                            tile.set_magnification(golem::TextureFilter::Nearest)
                                .unwrap();
                            *image = Some(tile);
                            accumulator.push((region, (xx, yy)));
                        }
                    }
                }
            }
        }

        for (r, t) in &background_tiles {
            if let Some(((_, t), _, _)) = self.tile_cache.get(t) {
                gfx.draw_image(t.as_ref().unwrap(), *r);
            }
        }
        for (r, t) in &terrain_tiles {
            if let Some((_, (_, t), _)) = self.tile_cache.get(t) {
                gfx.draw_image(t.as_ref().unwrap(), *r);
            }
        }

        for (sprite_id, sprite) in &self.sprites {
            let sx = sprite.loc.x - x as f32;
            let sy = sprite.loc.y - y as f32;
            let w = (SPRITE_WIDTH as u32 * sprite.x_scale) as f32;
            let h = (SPRITE_WIDTH as u32 * sprite.y_scale) as f32;
            if sx > -w && sx < width as f32 && sy > -h && sy < height as f32 {
                if !self.sprite_cache.contains_key(sprite_id) {
                    self.sprite_cache.insert(*sprite_id, sprite.image(gfx));
                }
                let sprite_image = &self.sprite_cache[sprite_id];
                let region = Rectangle::new(
                    Vector::new(
                        ((sprite.loc.x as i32 - x) as f32 / scale).floor(),
                        ((sprite.loc.y as i32 - y) as f32 / scale).floor(),
                    ),
                    Vector::new((w / scale).ceil(), (h / scale).ceil()),
                );
                gfx.draw_image(sprite_image, region);
                if let Some(t) = sprite.potion_timer {
                    if t > 0.0 {
                        let red_shift: u8 = ((t
                            * (10.0 + ((SCALE_CHANGE_TIMEOUT - t) / SCALE_CHANGE_TIMEOUT) * 20.0)
                                .sin()
                            + 1.0)
                            * 255.0) as u8;
                        let mut pixels = [0; SPRITE_WIDTH * SPRITE_WIDTH * 4];
                        for x in 0..SPRITE_WIDTH {
                            for y in 0..SPRITE_WIDTH {
                                let i = (x + y * SPRITE_WIDTH as usize);
                                if sprite.collider[i] {
                                    pixels[i * 4] = red_shift;
                                    pixels[i * 4 + 1] = 0xff;
                                    pixels[i * 4 + 1] = 0xff;
                                    pixels[i * 4 + 3] = 100;
                                }
                            }
                        }
                        let mut overlay = Image::from_raw(
                            gfx,
                            Some(&pixels),
                            SPRITE_WIDTH as u32,
                            SPRITE_WIDTH as u32,
                            PixelFormat::RGBA,
                        )
                        .unwrap();
                        overlay
                            .set_magnification(golem::TextureFilter::Nearest)
                            .unwrap();
                        gfx.draw_image(&overlay, region);
                    }
                }
            }
        }

        for (r, t) in &foreground_tiles {
            if let Some((_, _, (_, t))) = self.tile_cache.get(t) {
                gfx.draw_image(t.as_ref().unwrap(), *r);
            }
        }
    }
}

enum TerrainChunk {
    Foreground(Sprite),
    Background(Sprite),
    Terrain(Sprite),
}
impl TerrainChunk {
    fn loc(&self) -> Vector {
        match self {
            TerrainChunk::Foreground(s) => s.loc,
            TerrainChunk::Background(s) => s.loc,
            TerrainChunk::Terrain(s) => s.loc,
        }
    }

    fn pixel_count(&self) -> u32 {
        match self {
            TerrainChunk::Foreground(s) => {
                s.x_scale * SPRITE_WIDTH as u32 + s.y_scale * SPRITE_WIDTH as u32
            }
            TerrainChunk::Background(s) => {
                s.x_scale * SPRITE_WIDTH as u32 + s.y_scale * SPRITE_WIDTH as u32
            }
            TerrainChunk::Terrain(s) => {
                s.x_scale * SPRITE_WIDTH as u32 + s.y_scale * SPRITE_WIDTH as u32
            }
        }
    }

    fn quarter(self) -> Vec<Self> {
        match self {
            TerrainChunk::Foreground(s) => s
                .quarter()
                .into_iter()
                .map(|s| TerrainChunk::Foreground(s))
                .collect(),
            TerrainChunk::Background(s) => s
                .quarter()
                .into_iter()
                .map(|s| TerrainChunk::Background(s))
                .collect(),
            TerrainChunk::Terrain(s) => s
                .quarter()
                .into_iter()
                .map(|s| TerrainChunk::Terrain(s))
                .collect(),
        }
    }
}

async fn app(window: Window, mut gfx: Graphics, mut input: Input) -> Result<()> {
    let sprites = image::load(std::io::Cursor::new(SPRITES), image::ImageFormat::Png).unwrap();
    //let map_data = include_bytes!("../static/map.tmx").to_vec();//quicksilver::load_file("map.tmx").await.expect("The file was not found!");
    let map_data = quicksilver::load_file("map.tmx")
        .await
        .expect("The file was not found!");
    let map = tiled::parse(&*map_data).unwrap();
    let mut scene = Scene::new();
    let mut player_id = None;
    let mut negative_terrain = vec![];
    let mut terrain_locations: HashSet<(u32, i32, i32)> = HashSet::default();
    let mut terrain_chunks = vec![];
    for group in &map.object_groups {
        if !group.visible {
            continue;
        }
        for object in &group.objects {
            let x_scale = (object.width / 16.0) as u32;
            let y_scale = (object.height / 16.0) as u32;
            assert_eq!(
                x_scale as f32 * 16.0,
                object.width,
                "badly scaled sprite {} in {}",
                object.id,
                group.name
            );
            assert_eq!(y_scale as f32 * 16.0, object.height);
            let flipped = object.gid & 0x80000000 != 0;
            let gid = object.gid & !0x80000000;
            let ty = (gid - 1) / 48;
            let tx = (gid - 1) - ty as u32 * 48;
            let gravity = if let Some(v) = object.properties.get("gravity") {
                match v {
                    tiled::PropertyValue::BoolValue(v) => *v,
                    _ => true,
                }
            } else {
                true
            };

            let preload = if let Some(v) = object.properties.get("preload") {
                match v {
                    tiled::PropertyValue::BoolValue(v) => *v,
                    _ => false,
                }
            } else {
                false
            };

            if group.name == "player" || group.name == "test_player" {
                player_id = Some(scene.add_character(Sprite::new(
                    &sprites,
                    tx as usize,
                    ty as usize,
                    object.x,
                    object.y - object.height,
                    x_scale,
                    y_scale,
                    Color::BLUE,
                )));
            } else if group.name == "collectable" {
                let mut collectable = Sprite::new(
                    &sprites,
                    tx as usize,
                    ty as usize,
                    object.x,
                    object.y - object.height,
                    x_scale,
                    y_scale,
                    Color::from_rgba(219, 242, 40, 1.0),
                )
                .maybe_flip(flipped);
                if !gravity {
                    collectable.gravity = false;
                }
                scene.add_collectable(collectable);
            } else if group.name == "objects" {
                let (potion_type, color) = if object.properties.contains_key("x_absolute")
                    || object.properties.contains_key("y_absolute")
                {
                    let x_absolute = if let Some(v) = object.properties.get("x_absolute") {
                        match v {
                            tiled::PropertyValue::IntValue(v) => Some(*v),
                            _ => None,
                        }
                    } else {
                        None
                    };
                    let y_absolute = if let Some(v) = object.properties.get("y_absolute") {
                        match v {
                            tiled::PropertyValue::IntValue(v) => Some(*v),
                            _ => None,
                        }
                    } else {
                        None
                    };
                    (PotionType::Absolute(x_absolute, y_absolute), Color::RED)
                } else {
                    let x_relative = if let Some(v) = object.properties.get("x_delta") {
                        match v {
                            tiled::PropertyValue::IntValue(v) => *v,
                            _ => 1,
                        }
                    } else {
                        1
                    };
                    let y_relative = if let Some(v) = object.properties.get("y_delta") {
                        match v {
                            tiled::PropertyValue::IntValue(v) => *v,
                            _ => 1,
                        }
                    } else {
                        1
                    };
                    let color = if x_relative + y_relative > 0 {
                        Color::RED
                    } else {
                        Color::BLUE
                    };
                    (PotionType::Relative(x_relative, y_relative), color)
                };
                let start_end = if let Some(v) = object.properties.get("start_end") {
                    match v {
                        tiled::PropertyValue::BoolValue(v) => *v,
                        _ => false,
                    }
                } else {
                    false
                };
                let mut potion = Sprite::new(
                    &sprites,
                    tx as usize,
                    ty as usize,
                    object.x,
                    object.y - object.height,
                    x_scale,
                    y_scale,
                    color,
                );
                if !gravity {
                    potion.gravity = false;
                }
                scene.add_potion(potion, potion_type, start_end);
            } else if group.name.starts_with("terrain") {
                if preload {
                    scene.add_terrain(
                        &Sprite::new(
                            &sprites,
                            tx as usize,
                            ty as usize,
                            object.x,
                            object.y - object.height,
                            x_scale,
                            y_scale,
                            Color::RED,
                        )
                        .maybe_flip(flipped),
                    );
                } else {
                    terrain_chunks.push(TerrainChunk::Terrain(
                        Sprite::new(
                            &sprites,
                            tx as usize,
                            ty as usize,
                            object.x,
                            object.y - object.height,
                            x_scale,
                            y_scale,
                            Color::RED,
                        )
                        .maybe_flip(flipped),
                    ));
                }
            } else if group.name.starts_with("negative-terrain") {
                //terrain_locations.insert((object.x as i32 / TILE_SIZE as i32, (object.y - object.height) as i32 / TILE_SIZE as i32));
                negative_terrain.push(
                    Sprite::new(
                        &sprites,
                        tx as usize,
                        ty as usize,
                        object.x,
                        object.y - object.height,
                        x_scale,
                        y_scale,
                        Color::RED,
                    )
                    .maybe_flip(flipped),
                );
            } else if group.name.starts_with("background") {
                if preload {
                    scene.add_background(
                        &Sprite::new(
                            &sprites,
                            tx as usize,
                            ty as usize,
                            object.x,
                            object.y - object.height,
                            x_scale,
                            y_scale,
                            Color::RED,
                        )
                        .maybe_flip(flipped),
                    );
                } else {
                    terrain_chunks.push(TerrainChunk::Background(
                        Sprite::new(
                            &sprites,
                            tx as usize,
                            ty as usize,
                            object.x,
                            object.y - object.height,
                            x_scale,
                            y_scale,
                            Color::RED,
                        )
                        .maybe_flip(flipped),
                    ));
                }
            } else if group.name.starts_with("foreground") {
                if preload {
                    scene.add_foreground(
                        &Sprite::new(
                            &sprites,
                            tx as usize,
                            ty as usize,
                            object.x,
                            object.y - object.height,
                            x_scale,
                            y_scale,
                            Color::RED,
                        )
                        .maybe_flip(flipped),
                    );
                } else {
                    terrain_chunks.push(TerrainChunk::Foreground(
                        Sprite::new(
                            &sprites,
                            tx as usize,
                            ty as usize,
                            object.x,
                            object.y - object.height,
                            x_scale,
                            y_scale,
                            Color::RED,
                        )
                        .maybe_flip(flipped),
                    ));
                }
            }
        }
    }
    let mut terrain_chunks: Vec<_> = terrain_chunks
        .into_iter()
        .flat_map(|c| {
            let mut result = vec![c];
            while result[0].pixel_count() > 160 * 160 {
                result = result.into_iter().flat_map(|c| c.quarter()).collect();
            }
            result
        })
        .collect();
    for terrain in negative_terrain {
        scene.clear_terrain(terrain);
    }

    for (sprite_id, sprite) in &scene.sprites {
        scene.sprite_cache.insert(*sprite_id, sprite.image(&gfx));
    }

    let mut terrain_locations: Vec<_> = terrain_locations
        .into_iter()
        .map(|(l, x, y)| (l, Vector::new(x as f32, y as f32)))
        .collect();

    let player_id = player_id.unwrap();

    let mut camera = Vector::new(0.0, 0.0);
    let mut camera_scale = 8.0;
    let mut setup_end = false;
    {
        let player = scene.sprites.get_mut(&player_id).unwrap();
        player.is_player = true;
        camera.x = player.loc.x;
        camera.y = player.loc.y;
        camera_scale = player.x_scale.max(player.y_scale) as f32;
        //player.collider[2 + 4 * SPRITE_WIDTH] = false;
        //player.collider[SPRITE_WIDTH-3 + 4 * SPRITE_WIDTH] = false;
    }

    let mut fps = 60.0;

    let mut update_timer = Timer::time_per_second(fps);
    let mut draw_timer = Timer::time_per_second(fps);
    let mut moving_left = false;
    let mut moving_right = false;

    let mut step_cache_warmer = |scene: &mut Scene, gfx: &mut Graphics, camera_scale: f32| {
        let mut did_work = false;
        if !terrain_chunks.is_empty() {
            let player_loc = scene.sprites[&player_id].loc;
            terrain_chunks.sort_by_key(|c| (player_loc.distance(c.loc()) * 10000.0) as i32);
            let mut pixel_budget = 512 * 512;
            while pixel_budget > 0 && !terrain_chunks.is_empty() {
                let chunk = terrain_chunks.pop().unwrap();
                pixel_budget -= chunk.pixel_count();
                match chunk {
                    TerrainChunk::Foreground(s) => scene.add_foreground(&s),
                    TerrainChunk::Background(s) => scene.add_background(&s),
                    TerrainChunk::Terrain(s) => scene.add_terrain(&s),
                }
                did_work = true;
            }
        }
        if !scene.tile_queue.is_empty() {
            did_work = true;
            let player_loc = scene.sprites[&player_id].loc / TILE_SIZE as f32;
            let mut min_idx = 0;
            let mut min_d = f32::INFINITY;
            let mut crash_priority = vec![];
            for (i, (_, x, y)) in scene.tile_queue.iter().enumerate() {
                let d = player_loc.distance(Vector::new(*x as f32, *y as f32));
                if d < (1300.0 * (camera_scale / 8.0)) / TILE_SIZE as f32 {
                    crash_priority.push(i);
                }
                if d < min_d {
                    min_d = d;
                    min_idx = i;
                }
            }
            if crash_priority.len() > 0 {}
            if crash_priority.len() < 3 && !crash_priority.contains(&min_idx) {
                crash_priority.push(min_idx);
            }
            /*
            terrain_locations.sort_by_key(|t| (player_loc.distance(t.1) * 10000.0) as i32);

            if !terrain_chunks.is_empty() && (terrain_locations[0].1).distance(player_loc) > 1920.0/camera_scale {
                return true
            }
            */
            crash_priority.sort();
            for i in crash_priority.into_iter().rev() {
                let (layer, x, y) = scene.tile_queue.swap_remove_index(i).unwrap();
                let e = {
                    let o = scene.tile_cache.entry((x, y)).or_default();
                    match layer {
                        0 => &mut o.0,
                        1 => &mut o.1,
                        _ => &mut o.2,
                    }
                };
                let (map, color) = match layer {
                    0 => (&mut scene.background_map, BACKGROUND_COLOR),
                    1 => (&mut scene.collision_map, TERRAIN_COLOR),
                    _ => (&mut scene.foreground_map, FOREGROUND_COLOR),
                };
                let tile =
                    e.0.get_or_insert_with(|| vec![0; (TILE_SIZE * TILE_SIZE * 4) as usize]);
                for dx in 0..TILE_SIZE {
                    for dy in 0..TILE_SIZE {
                        if map.check_point(
                            x * TILE_SIZE as i32 + dx as i32,
                            y * TILE_SIZE as i32 + dy as i32,
                        ) {
                            let i = (dx + dy * TILE_SIZE) as usize * 4;
                            tile[i] = (color.r * 255.0).clamp(0.0, 255.0) as u8;
                            tile[i + 1] = (color.g * 255.0).clamp(0.0, 255.0) as u8;
                            tile[i + 2] = (color.b * 255.0).clamp(0.0, 255.0) as u8;
                            tile[i + 3] = 255;
                        }
                    }
                }
                e.1 = None;
            }
        }
        did_work
    };
    /*
    for _ in 0..20 {
        step_cache_warmer(&mut scene, &mut gfx, camera_scale);
    }
    */

    let mut paused = false;
    loop {
        while let Some(e) = input.next_event().await {
            let player = scene.sprites.get_mut(&player_id).unwrap();
            match e {
                Event::GamepadAxis(e) => match e.axis() {
                    GamepadAxis::LeftStickX | GamepadAxis::RightStickX => {
                        if e.value() > 0.5 {
                            moving_right = true;
                            moving_left = false;
                        } else if e.value() < -0.5 {
                            moving_left = true;
                            moving_right = false;
                        } else {
                            moving_right = false;
                            moving_left = false;
                        }
                    }
                    _ => (),
                },
                Event::GamepadButton(e) => match e.button() {
                    GamepadButton::South => {
                        if e.is_down() {
                            if player.ground_contact && !paused {
                                player.jumping = true;
                                player.velocity.y = -80.0 / fps;
                            }
                        }
                    }
                    GamepadButton::DPadLeft => {
                        if e.is_down() {
                            moving_left = true;
                        } else {
                            moving_left = false;
                        }
                    }
                    GamepadButton::DPadRight => {
                        if e.is_down() {
                            moving_right = true;
                        } else {
                            moving_right = false;
                        }
                    }
                    GamepadButton::Start => {
                        if e.is_down() {
                            paused = !paused;
                        }
                    }
                    _ => (),
                },
                Event::KeyboardInput(e) => match e.key() {
                    Key::P => {
                        if e.is_down() {
                            paused = !paused;
                        }
                    }
                    Key::Right | Key::D => {
                        if e.is_down() {
                            moving_right = true;
                        } else {
                            moving_right = false;
                        }
                    }
                    Key::Left | Key::A => {
                        if e.is_down() {
                            moving_left = true;
                        } else {
                            moving_left = false;
                        }
                    }
                    Key::Up | Key::W => {
                        if e.is_down() {
                            if player.ground_contact && !paused {
                                player.jumping = true;
                                player.velocity.y = -80.0 / fps;
                            }
                        } else {
                            if !player.ground_contact && player.jumping {
                                player.velocity.y = player.velocity.y.max(-2.0);
                            }
                        }
                    }
                    Key::Q => {
                        std::process::exit(0);
                    }
                    _ => (),
                },
                _ => (),
            }
        }

        {
            let player = scene.sprites.get_mut(&player_id).unwrap();
            let vx = if input.key_down(Key::LShift) && player.ground_contact {
                130.0
            } else {
                60.0
            };
            if moving_right {
                player.velocity.x = vx / fps;
            } else if moving_left {
                player.velocity.x = -vx / fps;
            } else {
                player.velocity.x = 0.0;
            }
        }
        //while update_timer.tick() && !paused {
        if update_timer.exhaust().is_some() {
            let player_loc = scene.sprites.get_mut(&player_id).unwrap().loc;
            if let Some(timer) = scene.sprites.get(&player_id).unwrap().potion_timer {
                if timer < 0.0 {
                    fps = 60.0;
                } else {
                    fps = 60.0;
                }
            } else {
                fps = 60.0;
            }
            scene.step_physics(player_loc, camera_scale, fps);
            if scene.done && !setup_end {
                setup_end = true;
                scene.sprites.retain(|i, _| *i == player_id);
                scene.particles.clear();
                scene.collectables.clear();
                scene.potions.clear();
                scene.sprites.get_mut(&player_id).unwrap().loc = Vector::new(10000.0, 30000.0);
                for (i, mut collectable) in scene.collected.drain() {
                    collectable.gravity = false;
                    collectable.velocity = Vector::new(0.0, 0.0);
                    if collectable.x_scale < 30 {
                        collectable.x_scale = 50;
                        collectable.y_scale = 50;
                        let x = (i as f32 * 1000.0 + camera.x).sin() * 2000.0 + 4000.0;
                        let y = (i as f32 * 3000.0 + camera.y).sin() * 2000.0 + 4000.0;
                        collectable.loc = Vector::new(x, y);
                    }
                    scene.sprites.insert(i, collectable);
                }
                scene.collision_map.clear();
                scene.foreground_map.clear();
                scene.background_map.clear();
                scene.tile_cache.clear();
            }
        }
        step_cache_warmer(&mut scene, &mut gfx, camera_scale);
        if draw_timer.exhaust().is_some() {
            let player = scene.sprites.get_mut(&player_id).unwrap();
            if player.y_scale < MAX_SCALE as u32 && !scene.done {
                if camera.distance(player.loc) > player.x_scale.max(player.y_scale) as f32 * 10.0 {
                    camera.x = camera.x * 0.9 + (player.loc.x) * 0.1;
                    camera.y = camera.y * 0.9 + (player.loc.y) * 0.1;
                }
            } else {
                camera.x = camera.x * 0.9 + 5293.0 * 0.1;
                camera.y = camera.y * 0.9 + 5429.0 * 0.1;
            }
            if (camera_scale - player.x_scale.max(player.y_scale) as f32).abs() / camera_scale > 0.1
            {
                camera_scale = camera_scale * 0.9 + player.x_scale.max(player.y_scale) as f32 * 0.1;
            }
            if scene.done {
                camera_scale = camera_scale * 0.9 + 100.0 * 0.1;
            }
            gfx.clear(Color::BLACK);
            let scale = if camera_scale > 8.0 {
                (camera_scale / 8.0) as f32
            } else {
                1.0 / (8.0 / camera_scale) as f32
            };
            scene.draw(
                &mut gfx,
                camera.x as i32,
                camera.y as i32,
                1920,
                1080,
                scale,
            );
            if paused {
                gfx.fill_rect(
                    &Rectangle::new_sized(Vector::new(1920.0, 1080.0)),
                    Color::from_rgba(255, 255, 255, 0.4),
                );
            }
            gfx.present(&window)?;
        }
    }
}
