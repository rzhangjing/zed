//! 四叉树瓦片邻接：给定瓦片坐标，沿树上下行查找其东/西/南/北相邻瓦片。
//!
//! 提供层级零瓦片生成、跨反日子线的经度环绕查找，以及基于子位置
//! （西北/东北/西南/东南）递归分解的四个方向邻接查询。

/// 四叉树中的一个瓦片坐标。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TileCoord {
    /// 瓦片 X 坐标。
    pub x: u32,
    /// 瓦片 Y 坐标。
    pub y: u32,
    /// 细节层级（0 = 最粗）。
    pub level: u32,
}

impl TileCoord {
    /// 创建一个新的瓦片坐标。
    pub fn new(x: u32, y: u32, level: u32) -> Self {
        Self { x, y, level }
    }

    /// 返回父坐标（若为层级 0 则返回 None）。
    pub fn parent(&self) -> Option<TileCoord> {
        // 层级零无父；否则父坐标为子坐标整除 2、层级减一。
        if self.level == 0 {
            None
        } else {
            Some(TileCoord {
                x: self.x / 2,
                y: self.y / 2,
                level: self.level - 1,
            })
        }
    }

    /// 返回西北子节点（x*2, y*2, level+1）。
    pub fn northwest_child(&self) -> TileCoord {
        TileCoord::new(self.x * 2, self.y * 2, self.level + 1)
    }

    /// 返回东北子节点（x*2+1, y*2, level+1）。
    pub fn northeast_child(&self) -> TileCoord {
        TileCoord::new(self.x * 2 + 1, self.y * 2, self.level + 1)
    }

    /// 返回西南子节点（x*2, y*2+1, level+1）。
    pub fn southwest_child(&self) -> TileCoord {
        TileCoord::new(self.x * 2, self.y * 2 + 1, self.level + 1)
    }

    /// 返回东南子节点（x*2+1, y*2+1, level+1）。
    pub fn southeast_child(&self) -> TileCoord {
        TileCoord::new(self.x * 2 + 1, self.y * 2 + 1, self.level + 1)
    }

    /// 判断本瓦片相对于其父节点处于哪个子位置。
    /// 若 level == 0 则返回 None。
    fn child_position(&self) -> Option<ChildPosition> {
        if self.level == 0 {
            return None;
        }
        // 依坐标奇偶判定子象限：x 为奇数偏东、y 为奇数偏南。
        let is_east = self.x % 2 == 1;
        let is_south = self.y % 2 == 1;
        Some(match (is_east, is_south) {
            (false, false) => ChildPosition::Northwest,
            (true, false) => ChildPosition::Northeast,
            (false, true) => ChildPosition::Southwest,
            (true, true) => ChildPosition::Southeast,
        })
    }
}

/// 瓦片相对其父节点所处的子象限。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChildPosition {
    /// 西北（左上）。
    Northwest,
    /// 东北（右上）。
    Northeast,
    /// 西南（左下）。
    Southwest,
    /// 东南（右下）。
    Southeast,
}

/// 用于邻接计算的切分方案描述符。
#[derive(Debug, Clone)]
pub struct TilingSchemeDescriptor {
    /// 层级 0 时 X 方向的瓦片数。
    pub x_tiles_at_level_zero: u32,
    /// 层级 0 时 Y 方向的瓦片数。
    pub y_tiles_at_level_zero: u32,
}

impl TilingSchemeDescriptor {
    /// 创建一个新的切分方案描述符。
    pub fn new(x_tiles: u32, y_tiles: u32) -> Self {
        Self {
            x_tiles_at_level_zero: x_tiles,
            y_tiles_at_level_zero: y_tiles,
        }
    }

    /// 地理（Geographic）切分方案（层级 0 为 2x1）。
    pub fn geographic() -> Self {
        Self::new(2, 1)
    }

    /// Web Mercator 切分方案（层级 0 为 1x1）。
    pub fn web_mercator() -> Self {
        Self::new(1, 1)
    }
}

/// 为给定切分方案创建层级零的瓦片。
///
/// 遍历顺序为行优先（外层 y、内层 x），故结果自西北起先向东再向南排列。
pub fn create_level_zero_tiles(scheme: &TilingSchemeDescriptor) -> Vec<TileCoord> {
    let mut result = Vec::with_capacity(
        (scheme.x_tiles_at_level_zero * scheme.y_tiles_at_level_zero) as usize,
    );
    for y in 0..scheme.y_tiles_at_level_zero {
        // 行优先填充：外层纬度带 y、内层经度带 x，得到 NW→E→S 顺序。
        for x in 0..scheme.x_tiles_at_level_zero {
            result.push(TileCoord::new(x, y, 0));
        }
    }
    result
}

/// 在给定坐标处查找层级零瓦片，X 沿反日子线环绕。
///
/// 若 Y 越界（北极以北或南极以南）则返回 None。
pub fn find_level_zero_tile(
    scheme: &TilingSchemeDescriptor,
    level_zero_tiles: &[TileCoord],
    x: i32,
    y: i32,
) -> Option<TileCoord> {
    let x_tiles = scheme.x_tiles_at_level_zero as i32;
    let y_tiles = scheme.y_tiles_at_level_zero as i32;

    let mut wrapped_x = x;
    // 经度单步环绕：越过 [0, x_tiles) 边界时回绕一整圈（仅需处理 ±1 越界）。
    if wrapped_x < 0 {
        wrapped_x += x_tiles;
    } else if wrapped_x >= x_tiles {
        wrapped_x -= x_tiles;
    }

    // 纬度不环绕：越过极区（北以北/南以南）即判定无对应瓦片。
    if y < 0 || y >= y_tiles {
        return None;
    }

    level_zero_tiles
        .iter()
        .find(|t| t.x == wrapped_x as u32 && t.y == y as u32)
        .copied()
}

/// 查找给定瓦片西侧的瓦片。
///
/// 若处于层级零则直接按 x-1 环绕查找；否则据本瓦片在父节点中的
/// 子位置递归：位于父节点东半侧的瓦片，其西邻即父节点的西邻。
pub fn find_tile_to_west(
    scheme: &TilingSchemeDescriptor,
    level_zero_tiles: &[TileCoord],
    tile: &TileCoord,
) -> Option<TileCoord> {
    let parent = match tile.parent() {
        None => {
            // 层级零无父：退化为按 x-1 环绕的层级零查找。
            return find_level_zero_tile(scheme, level_zero_tiles, tile.x as i32 - 1, tile.y as i32)
        }
        Some(p) => p,
    };

    match tile.child_position() {
        // 本瓦片居父节点东半侧：其西邻是同父的西半侧兄弟，一步可达。
        Some(ChildPosition::Southeast) => Some(parent.southwest_child()),
        Some(ChildPosition::Northeast) => Some(parent.northwest_child()),
        Some(ChildPosition::Southwest) | Some(ChildPosition::Northwest) => {
            // 本瓦片居父节点西半侧：需先递归求父之西邻，再取其子瓦片。
            let west_of_parent = find_tile_to_west(scheme, level_zero_tiles, &parent)?;
            match tile.child_position() {
                Some(ChildPosition::Southwest) => Some(west_of_parent.southeast_child()),
                _ => Some(west_of_parent.northeast_child()),
            }
        }
        None => unreachable!(),
    }
}

/// 查找给定瓦片东侧的瓦片。
///
/// 若处于层级零则直接按 x+1 环绕查找；否则据本瓦片在父节点中的
/// 子位置递归：位于父节点西半侧的瓦片，其东邻即父节点的东邻。
pub fn find_tile_to_east(
    scheme: &TilingSchemeDescriptor,
    level_zero_tiles: &[TileCoord],
    tile: &TileCoord,
) -> Option<TileCoord> {
    let parent = match tile.parent() {
        None => {
            // 层级零无父：退化为按 x+1 环绕的层级零查找。
            return find_level_zero_tile(scheme, level_zero_tiles, tile.x as i32 + 1, tile.y as i32)
        }
        Some(p) => p,
    };

    match tile.child_position() {
        // 本瓦片居父节点西半侧：其东邻是同父的东半侧兄弟，一步可达。
        Some(ChildPosition::Southwest) => Some(parent.southeast_child()),
        Some(ChildPosition::Northwest) => Some(parent.northeast_child()),
        Some(ChildPosition::Southeast) | Some(ChildPosition::Northeast) => {
            // 本瓦片居父节点东半侧：需先递归求父之东邻，再取其子瓦片。
            let east_of_parent = find_tile_to_east(scheme, level_zero_tiles, &parent)?;
            match tile.child_position() {
                Some(ChildPosition::Southeast) => Some(east_of_parent.southwest_child()),
                _ => Some(east_of_parent.northwest_child()),
            }
        }
        None => unreachable!(),
    }
}

/// 查找给定瓦片南侧的瓦片。
///
/// 若处于层级零则按 y+1 查找（不环绕）；否则据本瓦片在父节点中
/// 处于北半侧还是南半侧递归地定位其南邻。
pub fn find_tile_to_south(
    scheme: &TilingSchemeDescriptor,
    level_zero_tiles: &[TileCoord],
    tile: &TileCoord,
) -> Option<TileCoord> {
    let parent = match tile.parent() {
        None => {
            // 层级零无父：按 y+1 查找（纬度不环绕）。
            return find_level_zero_tile(scheme, level_zero_tiles, tile.x as i32, tile.y as i32 + 1)
        }
        Some(p) => p,
    };

    match tile.child_position() {
        // 本瓦片居父节点北半侧：其南邻是同父的南半侧兄弟，一步可达。
        Some(ChildPosition::Northwest) => Some(parent.southwest_child()),
        Some(ChildPosition::Northeast) => Some(parent.southeast_child()),
        Some(ChildPosition::Southwest) | Some(ChildPosition::Southeast) => {
            // 本瓦片居父节点南半侧：需先递归求父之南邻，再取其子瓦片。
            let south_of_parent = find_tile_to_south(scheme, level_zero_tiles, &parent)?;
            match tile.child_position() {
                Some(ChildPosition::Southwest) => Some(south_of_parent.northwest_child()),
                _ => Some(south_of_parent.northeast_child()),
            }
        }
        None => unreachable!(),
    }
}

/// 查找给定瓦片北侧的瓦片。
///
/// 若处于层级零则按 y-1 查找（不环绕）；否则据本瓦片在父节点中
/// 处于南半侧还是北半侧递归地定位其北邻。
pub fn find_tile_to_north(
    scheme: &TilingSchemeDescriptor,
    level_zero_tiles: &[TileCoord],
    tile: &TileCoord,
) -> Option<TileCoord> {
    let parent = match tile.parent() {
        None => {
            // 层级零无父：按 y-1 查找（纬度不环绕）。
            return find_level_zero_tile(scheme, level_zero_tiles, tile.x as i32, tile.y as i32 - 1)
        }
        Some(p) => p,
    };

    match tile.child_position() {
        // 本瓦片居父节点南半侧：其北邻是同父的北半侧兄弟，一步可达。
        Some(ChildPosition::Southwest) => Some(parent.northwest_child()),
        Some(ChildPosition::Southeast) => Some(parent.northeast_child()),
        Some(ChildPosition::Northwest) | Some(ChildPosition::Northeast) => {
            // 本瓦片居父节点北半侧：需先递归求父之北邻，再取其子瓦片。
            let north_of_parent = find_tile_to_north(scheme, level_zero_tiles, &parent)?;
            match tile.child_position() {
                Some(ChildPosition::Northwest) => Some(north_of_parent.southwest_child()),
                _ => Some(north_of_parent.southeast_child()),
            }
        }
        None => unreachable!(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_level_zero_tiles_geographic() {
        let scheme = TilingSchemeDescriptor::new(2, 1);
        let tiles = create_level_zero_tiles(&scheme);
        assert_eq!(tiles.len(), 2);
        assert_eq!(tiles[0], TileCoord::new(0, 0, 0));
        assert_eq!(tiles[1], TileCoord::new(1, 0, 0));
    }

    #[test]
    fn test_create_level_zero_tiles_3x3() {
        let scheme = TilingSchemeDescriptor::new(3, 3);
        let tiles = create_level_zero_tiles(&scheme);
        assert_eq!(tiles.len(), 9);
        // 按 NW→E→S 排序
        assert_eq!(tiles[0], TileCoord::new(0, 0, 0));
        assert_eq!(tiles[1], TileCoord::new(1, 0, 0));
        assert_eq!(tiles[2], TileCoord::new(2, 0, 0));
        assert_eq!(tiles[3], TileCoord::new(0, 1, 0));
    }

    #[test]
    fn test_find_level_zero_tile_wraps_x() {
        let scheme = TilingSchemeDescriptor::new(3, 3);
        let tiles = create_level_zero_tiles(&scheme);
        // x=-1 环绕为 x=2
        let found = find_level_zero_tile(&scheme, &tiles, -1, 0);
        assert_eq!(found, Some(TileCoord::new(2, 0, 0)));
        // x=3 环绕为 x=0
        let found = find_level_zero_tile(&scheme, &tiles, 3, 0);
        assert_eq!(found, Some(TileCoord::new(0, 0, 0)));
    }

    #[test]
    fn test_find_level_zero_tile_y_out_of_bounds() {
        let scheme = TilingSchemeDescriptor::new(3, 3);
        let tiles = create_level_zero_tiles(&scheme);
        assert_eq!(find_level_zero_tile(&scheme, &tiles, 0, -1), None);
        assert_eq!(find_level_zero_tile(&scheme, &tiles, 0, 3), None);
    }

    #[test]
    fn test_adjacency_level_zero() {
        let scheme = TilingSchemeDescriptor::new(3, 3);
        let tiles = create_level_zero_tiles(&scheme);
        let tile = TileCoord::new(0, 0, 0);

        // 西侧环绕
        assert_eq!(
            find_tile_to_west(&scheme, &tiles, &tile),
            Some(TileCoord::new(2, 0, 0))
        );
        // 东
        assert_eq!(
            find_tile_to_east(&scheme, &tiles, &tile),
            Some(TileCoord::new(1, 0, 0))
        );
        // 第 0 行的北侧 → None
        assert_eq!(find_tile_to_north(&scheme, &tiles, &tile), None);
        // 南
        assert_eq!(
            find_tile_to_south(&scheme, &tiles, &tile),
            Some(TileCoord::new(0, 1, 0))
        );
    }
}
