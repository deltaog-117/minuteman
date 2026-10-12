// Minuteman - a fast, Ranger-inspired terminal file manager
// Copyright (C) 2026  Davi Oliveira Gonçalves
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published
// by the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

//! The treemap behind the atlas (`state`, `canvas`): a list of sizes laid out as rectangles of
//! terminal cells whose areas follow the sizes, and the moves between those rectangles.
//!
//! Everything here is pure and works on whole cells, so a tile is never half a cell and the tiles
//! cover the map exactly. The layout is a balanced binary split: the entries (largest first) are
//! cut into two groups of about equal weight, the map is cut across its longer side in the same
//! proportion, and each half is laid out the same way. A cell is about twice as tall as it is
//! wide, so a side's length is compared with the other counted double, which keeps tiles close to
//! square on screen. A squarified layout would give slightly squarer tiles, but cutting whole
//! cells at integer positions is what makes "every tile is at least one cell and none overlap"
//! true by construction rather than by rounding luck.
//!
//! A group whose share of the map comes to fewer than `MIN_CELLS` cells is not worth a tile of
//! its own, nor is a map too small to cut: its entries are folded into one `Item::Small` tile,
//! which is drawn but never chosen.

/// A rectangle of canvas cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

impl Rect {
    pub const fn new(x: u16, y: u16, width: u16, height: u16) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn right(self) -> u16 {
        self.x.saturating_add(self.width)
    }

    pub fn bottom(self) -> u16 {
        self.y.saturating_add(self.height)
    }

    pub fn contains(self, x: u16, y: u16) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }
}

/// The fewest cells an entry (or a group of them) may deserve and still get a tile of its own.
pub const MIN_CELLS: u128 = 6;

/// What a tile stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Item {
    /// The entry at this position in the weights given.
    Entry(usize),
    /// This many entries too small to show one by one, folded together.
    Small(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tile {
    pub item: Item,
    pub rect: Rect,
}

/// A direction to move in across the map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Left,
    Right,
    Up,
    Down,
}

struct Plan<'a> {
    /// Positions in the weights given of the entries that have a weight, in the order given.
    entries: &'a [usize],
    /// `prefix[n]` is the weight of the first `n` of `entries`.
    prefix: &'a [u128],
    cells: u128,
}

/// Lays `weights` out in `area`: one `Entry` tile per entry that deserves one, in the order
/// given, plus at most a few `Small` tiles for the rest. Entries with a weight of zero are not
/// drawn. Pass the largest first for the best picture. The tiles never overlap, stay inside
/// `area`, and together cover all of it (unless there is nothing to show).
pub fn treemap(weights: &[u64], area: Rect) -> Vec<Tile> {
    let entries: Vec<usize> = (0..weights.len()).filter(|&i| weights[i] > 0).collect();
    if entries.is_empty() || area.width == 0 || area.height == 0 {
        return Vec::new();
    }
    let mut prefix = Vec::with_capacity(entries.len() + 1);
    prefix.push(0u128);
    let mut sum = 0u128;
    for &i in &entries {
        sum += u128::from(weights[i]);
        prefix.push(sum);
    }
    let plan = Plan {
        entries: &entries,
        prefix: &prefix,
        cells: u128::from(area.width) * u128::from(area.height),
    };
    let mut tiles = Vec::new();
    split(&plan, 0, entries.len(), area, &mut tiles);
    tiles
}

fn split(plan: &Plan<'_>, lo: usize, hi: usize, rect: Rect, out: &mut Vec<Tile>) {
    let count = hi - lo;
    let total = plan.prefix[plan.entries.len()];
    let weight = plan.prefix[hi] - plan.prefix[lo];
    // What this group's share of the whole map comes to, in cells. Measured against the whole map
    // and not the group's own rectangle, so a lone small entry is folded however it got there.
    let ideal = weight * plan.cells / total;
    let floor = MIN_CELLS.min(plan.cells);
    if ideal < floor {
        out.push(Tile {
            item: Item::Small(count),
            rect,
        });
        return;
    }
    if count == 1 {
        out.push(Tile {
            item: Item::Entry(plan.entries[lo]),
            rect,
        });
        return;
    }
    // Across the longer side as the eye sees it; the other one if that side is a single cell.
    let wide = u32::from(rect.width) >= 2 * u32::from(rect.height);
    let across_width = match (wide, rect.width >= 2, rect.height >= 2) {
        (_, false, false) => {
            out.push(Tile {
                item: Item::Small(count),
                rect,
            });
            return;
        }
        (true, true, _) | (false, true, false) => true,
        _ => false,
    };

    // The first position whose left side holds at least half the weight, or the one before it if
    // that is closer, keeping both groups non-empty.
    let base = plan.prefix[lo];
    let at = plan.prefix[lo + 1..hi].partition_point(|&p| 2 * (p - base) < weight) + lo + 1;
    let mid = at.min(hi - 1);
    let mid = if mid > lo + 1 {
        let (after, before) = (plan.prefix[mid] - base, plan.prefix[mid - 1] - base);
        if (2 * after).abs_diff(weight) <= (2 * before).abs_diff(weight) {
            mid
        } else {
            mid - 1
        }
    } else {
        mid
    };

    let left_weight = plan.prefix[mid] - base;
    let length = if across_width {
        rect.width
    } else {
        rect.height
    };
    let cut = (u128::from(length) * left_weight * 2 + weight) / (2 * weight);
    // Both halves keep at least one cell of the side being cut. The clamp is `[1, length - 1]`
    // and `length >= 2` was settled above, so it cannot invert.
    let cut = u16::try_from(cut).unwrap_or(length).clamp(1, length - 1);
    let (first, second) = if across_width {
        (
            Rect::new(rect.x, rect.y, cut, rect.height),
            Rect::new(rect.x + cut, rect.y, rect.width - cut, rect.height),
        )
    } else {
        (
            Rect::new(rect.x, rect.y, rect.width, cut),
            Rect::new(rect.x, rect.y + cut, rect.width, rect.height - cut),
        )
    };
    split(plan, lo, mid, first, out);
    split(plan, mid, hi, second, out);
}

/// The tile under `pointer`, if any.
pub fn tile_at(tiles: &[Tile], x: u16, y: u16) -> Option<&Tile> {
    tiles.iter().find(|tile| tile.rect.contains(x, y))
}

/// Where a tile's middle is, in half-cells across and quarter-rows down so that one unit is the
/// same distance on screen both ways (a cell is about twice as tall as wide).
fn middle(rect: Rect) -> (i64, i64) {
    (
        2 * i64::from(rect.x) + i64::from(rect.width),
        2 * (2 * i64::from(rect.y) + i64::from(rect.height)),
    )
}

/// The entry to move to from entry `from` in direction `dir`: the nearest tile whose middle lies
/// that way, counting sideways drift double. `None` when there is none. If `from` has no tile of
/// its own (it was folded into a small tile), the first entry on the map, so a move always
/// lands somewhere.
pub fn neighbor(tiles: &[Tile], from: usize, dir: Dir) -> Option<usize> {
    let entry = |tile: &Tile| match tile.item {
        Item::Entry(i) => Some(i),
        Item::Small(_) => None,
    };
    let Some(origin) = tiles.iter().find(|t| t.item == Item::Entry(from)) else {
        return tiles.iter().find_map(entry);
    };
    let (ox, oy) = middle(origin.rect);
    tiles
        .iter()
        .filter_map(|tile| {
            let i = entry(tile)?;
            let (x, y) = middle(tile.rect);
            let (along, across) = match dir {
                Dir::Right => (x - ox, y - oy),
                Dir::Left => (ox - x, y - oy),
                Dir::Down => (y - oy, x - ox),
                Dir::Up => (oy - y, x - ox),
            };
            (along > 0).then_some((along + 2 * across.abs(), i))
        })
        .min()
        .map(|(_, i)| i)
}

/// The entry after (`step` positive) or before (`step` negative) entry `from` among those with a
/// tile, in the order given, wrapping round. Visits every tile in turn, so nothing is out of
/// reach however the map falls.
pub fn cycle(tiles: &[Tile], from: usize, step: isize) -> Option<usize> {
    let mut shown: Vec<usize> = tiles
        .iter()
        .filter_map(|tile| match tile.item {
            Item::Entry(i) => Some(i),
            Item::Small(_) => None,
        })
        .collect();
    shown.sort_unstable();
    if shown.is_empty() {
        return None;
    }
    let len = shown.len();
    let here = shown.iter().position(|&i| i >= from).unwrap_or(0);
    let landed = if step >= 0 {
        // Past `from` itself when it has a tile.
        let first = if shown.get(here) == Some(&from) {
            here + 1
        } else {
            here
        };
        first + step.unsigned_abs() - 1
    } else {
        here + len - step.unsigned_abs()
    };
    shown.get(landed % len).copied()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn area_of(rect: Rect) -> u64 {
        u64::from(rect.width) * u64::from(rect.height)
    }

    fn overlaps(a: Rect, b: Rect) -> bool {
        a.x < b.right() && b.x < a.right() && a.y < b.bottom() && b.y < a.bottom()
    }

    fn entries(tiles: &[Tile]) -> Vec<usize> {
        tiles
            .iter()
            .filter_map(|t| match t.item {
                Item::Entry(i) => Some(i),
                Item::Small(_) => None,
            })
            .collect()
    }

    #[test]
    fn nothing_to_show_gives_no_tiles() {
        let area = Rect::new(0, 0, 40, 10);
        assert!(treemap(&[], area).is_empty());
        assert!(treemap(&[0, 0], area).is_empty());
        assert!(treemap(&[5], Rect::new(0, 0, 0, 10)).is_empty());
        assert!(treemap(&[5], Rect::new(0, 0, 10, 0)).is_empty());
    }

    #[test]
    fn a_lone_entry_fills_the_map_even_when_the_map_is_tiny() {
        for (w, h) in [(1, 1), (2, 1), (1, 2), (3, 2), (40, 10)] {
            let area = Rect::new(3, 4, w, h);
            let tiles = treemap(&[7], area);
            assert_eq!(
                tiles,
                [Tile {
                    item: Item::Entry(0),
                    rect: area
                }]
            );
        }
    }

    #[test]
    fn areas_follow_the_sizes() {
        // Three to one on a wide map: cut across the width, at three quarters.
        let tiles = treemap(&[300, 100], Rect::new(0, 0, 40, 10));
        assert_eq!(tiles.len(), 2);
        assert_eq!(tiles[0].rect, Rect::new(0, 0, 30, 10));
        assert_eq!(tiles[1].rect, Rect::new(30, 0, 10, 10));
    }

    #[test]
    fn a_tall_map_is_cut_across_its_height() {
        // 10 wide and 20 tall is, to the eye, as wide as it is tall: cut the height first.
        let tiles = treemap(&[1, 1], Rect::new(0, 0, 10, 20));
        assert_eq!(tiles[0].rect, Rect::new(0, 0, 10, 10));
        assert_eq!(tiles[1].rect, Rect::new(0, 10, 10, 10));
    }

    #[test]
    fn entries_too_small_to_show_are_folded_into_one_tile() {
        // One big entry and a hundred specks.
        let mut weights = vec![1_000_000u64];
        weights.extend(std::iter::repeat_n(1, 100));
        let tiles = treemap(&weights, Rect::new(0, 0, 40, 10));
        assert_eq!(entries(&tiles), [0]);
        let folded: usize = tiles
            .iter()
            .filter_map(|t| match t.item {
                Item::Small(n) => Some(n),
                Item::Entry(_) => None,
            })
            .sum();
        assert_eq!(folded, 100);
    }

    #[test]
    fn zero_sized_entries_get_no_tile_and_keep_the_others_positions() {
        let tiles = treemap(&[10, 0, 10], Rect::new(0, 0, 20, 10));
        assert_eq!(entries(&tiles), [0, 2]);
    }

    fn tile(i: usize, x: u16, y: u16, w: u16, h: u16) -> Tile {
        Tile {
            item: Item::Entry(i),
            rect: Rect::new(x, y, w, h),
        }
    }

    #[test]
    fn moving_goes_to_the_nearest_tile_that_way() {
        // 0 | 1
        // -----
        // 2 | 3
        let tiles = [
            tile(0, 0, 0, 10, 5),
            tile(1, 10, 0, 10, 5),
            tile(2, 0, 5, 10, 5),
            tile(3, 10, 5, 10, 5),
        ];
        assert_eq!(neighbor(&tiles, 0, Dir::Right), Some(1));
        assert_eq!(neighbor(&tiles, 0, Dir::Down), Some(2));
        assert_eq!(neighbor(&tiles, 3, Dir::Left), Some(2));
        assert_eq!(neighbor(&tiles, 3, Dir::Up), Some(1));
        assert_eq!(neighbor(&tiles, 0, Dir::Left), None);
        assert_eq!(neighbor(&tiles, 0, Dir::Up), None);
    }

    #[test]
    fn moving_from_an_entry_with_no_tile_lands_on_the_first_one() {
        let tiles = [tile(4, 0, 0, 10, 5), tile(7, 10, 0, 10, 5)];
        assert_eq!(neighbor(&tiles, 9, Dir::Right), Some(4));
        assert_eq!(neighbor(&[], 0, Dir::Right), None);
    }

    #[test]
    fn a_small_tile_is_never_moved_to() {
        let tiles = [
            tile(0, 0, 0, 10, 5),
            Tile {
                item: Item::Small(3),
                rect: Rect::new(10, 0, 10, 5),
            },
        ];
        assert_eq!(neighbor(&tiles, 0, Dir::Right), None);
    }

    #[test]
    fn cycling_wraps_in_both_directions() {
        let tiles = [
            tile(2, 0, 0, 5, 5),
            tile(5, 5, 0, 5, 5),
            tile(9, 10, 0, 5, 5),
        ];
        assert_eq!(cycle(&tiles, 2, 1), Some(5));
        assert_eq!(cycle(&tiles, 9, 1), Some(2));
        assert_eq!(cycle(&tiles, 2, -1), Some(9));
        assert_eq!(cycle(&tiles, 5, -1), Some(2));
        // From an entry that has no tile: the next one along either way.
        assert_eq!(cycle(&tiles, 3, 1), Some(5));
        assert_eq!(cycle(&tiles, 3, -1), Some(2));
        assert_eq!(cycle(&[], 0, 1), None);
    }

    fn weights_and_area() -> impl Strategy<Value = (Vec<u64>, Rect)> {
        (
            prop::collection::vec(
                prop_oneof![
                    3 => 0u64..1_000_000_000_000,
                    1 => Just(0u64),
                    1 => 1u64..50,
                ],
                0..60,
            ),
            (0u16..160, 0u16..50),
        )
            .prop_map(|(mut weights, (w, h))| {
                weights.sort_unstable_by(|a, b| b.cmp(a));
                (weights, Rect::new(7, 3, w, h))
            })
    }

    proptest! {
        /// However the sizes and the map fall, the tiles stay inside the map, never overlap, are
        /// never empty, and cover the map to the last cell whenever there is anything to show.
        #[test]
        fn tiles_partition_the_map_exactly((weights, area) in weights_and_area()) {
            let tiles = treemap(&weights, area);
            let shown = weights.iter().any(|&w| w > 0) && area.width > 0 && area.height > 0;
            if !shown {
                prop_assert!(tiles.is_empty());
                return Ok(());
            }
            let mut covered = 0u64;
            for (i, a) in tiles.iter().enumerate() {
                prop_assert!(a.rect.width >= 1 && a.rect.height >= 1);
                prop_assert!(area.contains(a.rect.x, a.rect.y));
                prop_assert!(a.rect.right() <= area.right() && a.rect.bottom() <= area.bottom());
                covered += area_of(a.rect);
                for b in &tiles[i + 1..] {
                    prop_assert!(!overlaps(a.rect, b.rect), "{a:?} overlaps {b:?}");
                }
            }
            prop_assert_eq!(covered, area_of(area));
        }

        /// Every entry with a size is on the map exactly once, either as its own tile or inside
        /// a small one; an entry without a size never is.
        #[test]
        fn every_sized_entry_is_accounted_for_once((weights, area) in weights_and_area()) {
            let tiles = treemap(&weights, area);
            if tiles.is_empty() {
                return Ok(());
            }
            let own = entries(&tiles);
            let mut sorted = own.clone();
            sorted.sort_unstable();
            sorted.dedup();
            prop_assert_eq!(sorted.len(), own.len(), "an entry has two tiles");
            prop_assert!(own.iter().all(|&i| weights[i] > 0));
            let folded: usize = tiles.iter().filter_map(|t| match t.item {
                Item::Small(n) => Some(n),
                Item::Entry(_) => None,
            }).sum();
            let sized = weights.iter().filter(|&&w| w > 0).count();
            prop_assert_eq!(own.len() + folded, sized);
        }

        /// A tile that gets drawn by itself had a share of at least `MIN_CELLS` cells, or the
        /// whole map is smaller than that.
        #[test]
        fn a_tile_of_its_own_deserves_one((weights, area) in weights_and_area()) {
            let total: u128 = weights.iter().map(|&w| u128::from(w)).sum();
            let cells = u128::from(area.width) * u128::from(area.height);
            for tile in treemap(&weights, area) {
                if let Item::Entry(i) = tile.item {
                    let ideal = u128::from(weights[i]) * cells / total;
                    prop_assert!(ideal >= MIN_CELLS.min(cells));
                }
            }
        }

        /// A move lands on a different entry that has a tile, strictly in the direction asked,
        /// and cycling forward visits every entry that has a tile before coming back.
        #[test]
        fn moves_land_on_tiles_in_the_right_direction_and_cycling_reaches_all(
            (weights, area) in weights_and_area(),
            pick in 0usize..60,
        ) {
            let tiles = treemap(&weights, area);
            let own = entries(&tiles);
            let Some(&from) = own.get(pick % own.len().max(1)) else {
                return Ok(());
            };
            let origin = tiles.iter().find(|t| t.item == Item::Entry(from)).unwrap().rect;
            for dir in [Dir::Left, Dir::Right, Dir::Up, Dir::Down] {
                if let Some(to) = neighbor(&tiles, from, dir) {
                    prop_assert!(own.contains(&to) && to != from);
                    let landed = tiles.iter().find(|t| t.item == Item::Entry(to)).unwrap().rect;
                    let (a, b) = (middle(origin), middle(landed));
                    match dir {
                        Dir::Right => prop_assert!(b.0 > a.0),
                        Dir::Left => prop_assert!(b.0 < a.0),
                        Dir::Down => prop_assert!(b.1 > a.1),
                        Dir::Up => prop_assert!(b.1 < a.1),
                    }
                }
            }
            let mut at = from;
            let mut visited = vec![from];
            for _ in 0..own.len() {
                at = cycle(&tiles, at, 1).unwrap();
                visited.push(at);
            }
            visited.sort_unstable();
            visited.dedup();
            let mut all = own.clone();
            all.sort_unstable();
            prop_assert_eq!(visited, all);
        }
    }
}
