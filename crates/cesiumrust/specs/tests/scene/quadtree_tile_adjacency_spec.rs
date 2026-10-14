//! QuadtreeTile 邻接 specs - 瓦片邻居查找
//! 参考自 Scene/QuadtreeTileSpec（13 个 A 类测试）

use cesium_quadtree::quadtree_tile_adjacency::{
    create_level_zero_tiles, find_level_zero_tile, find_tile_to_east, find_tile_to_north,
    find_tile_to_south, find_tile_to_west, TileCoord, TilingSchemeDescriptor,
};

// ─── createLevelZeroTiles ───────────────────────────────────────────────────

#[test]
fn creates_expected_number_of_tiles() {
    let scheme = TilingSchemeDescriptor::new(3, 2);
    let tiles = create_level_zero_tiles(&scheme);
    assert_eq!(tiles.len(), 6);
}

#[test]
fn created_tiles_are_ordered_northwest_east_south() {
    let scheme = TilingSchemeDescriptor::new(3, 3);
    let tiles = create_level_zero_tiles(&scheme);
    // 第 0 行（北）：(0,0), (1,0), (2,0)
    assert_eq!(tiles[0], TileCoord::new(0, 0, 0));
    assert_eq!(tiles[1], TileCoord::new(1, 0, 0));
    assert_eq!(tiles[2], TileCoord::new(2, 0, 0));
    // 第 1 行：(0,1), (1,1), (2,1)
    assert_eq!(tiles[3], TileCoord::new(0, 1, 0));
    assert_eq!(tiles[4], TileCoord::new(1, 1, 0));
    assert_eq!(tiles[5], TileCoord::new(2, 1, 0));
    // 第 2 行（南）：(0,2), (1,2), (2,2)
    assert_eq!(tiles[6], TileCoord::new(0, 2, 0));
    assert_eq!(tiles[7], TileCoord::new(1, 2, 0));
    assert_eq!(tiles[8], TileCoord::new(2, 2, 0));
}

// ─── findLevelZeroTile ──────────────────────────────────────────────────────

#[test]
fn wraps_x_around_antimeridian() {
    let scheme = TilingSchemeDescriptor::new(3, 3);
    let tiles = create_level_zero_tiles(&scheme);

    // x=-1 环绕到 x=2
    assert_eq!(
        find_level_zero_tile(&scheme, &tiles, -1, 0),
        Some(TileCoord::new(2, 0, 0))
    );
    // x=3 环绕到 x=0
    assert_eq!(
        find_level_zero_tile(&scheme, &tiles, 3, 0),
        Some(TileCoord::new(0, 0, 0))
    );
}

#[test]
fn returns_none_for_y_out_of_bounds() {
    let scheme = TilingSchemeDescriptor::new(3, 3);
    let tiles = create_level_zero_tiles(&scheme);

    // 北极之北
    assert_eq!(find_level_zero_tile(&scheme, &tiles, 0, -1), None);
    // 南极之南
    assert_eq!(find_level_zero_tile(&scheme, &tiles, 0, 3), None);
}

// ─── 层级零的邻接（根瓦片） ───────────────────────────────────

#[test]
fn can_get_tiles_around_a_root_tile() {
    let scheme = TilingSchemeDescriptor::new(3, 3);
    let tiles = create_level_zero_tiles(&scheme);

    // L0X0Y0
    let t = TileCoord::new(0, 0, 0);
    assert_eq!(find_tile_to_west(&scheme, &tiles, &t), Some(TileCoord::new(2, 0, 0))); // 环绕
    assert_eq!(find_tile_to_east(&scheme, &tiles, &t), Some(TileCoord::new(1, 0, 0)));
    assert_eq!(find_tile_to_north(&scheme, &tiles, &t), None); // 北极
    assert_eq!(find_tile_to_south(&scheme, &tiles, &t), Some(TileCoord::new(0, 1, 0)));

    // L0X1Y0
    let t = TileCoord::new(1, 0, 0);
    assert_eq!(find_tile_to_west(&scheme, &tiles, &t), Some(TileCoord::new(0, 0, 0)));
    assert_eq!(find_tile_to_east(&scheme, &tiles, &t), Some(TileCoord::new(2, 0, 0)));
    assert_eq!(find_tile_to_north(&scheme, &tiles, &t), None);
    assert_eq!(find_tile_to_south(&scheme, &tiles, &t), Some(TileCoord::new(1, 1, 0)));

    // L0X2Y0
    let t = TileCoord::new(2, 0, 0);
    assert_eq!(find_tile_to_west(&scheme, &tiles, &t), Some(TileCoord::new(1, 0, 0)));
    assert_eq!(find_tile_to_east(&scheme, &tiles, &t), Some(TileCoord::new(0, 0, 0))); // 环绕
    assert_eq!(find_tile_to_north(&scheme, &tiles, &t), None);
    assert_eq!(find_tile_to_south(&scheme, &tiles, &t), Some(TileCoord::new(2, 1, 0)));

    // L0X0Y1
    let t = TileCoord::new(0, 1, 0);
    assert_eq!(find_tile_to_west(&scheme, &tiles, &t), Some(TileCoord::new(2, 1, 0))); // 环绕
    assert_eq!(find_tile_to_east(&scheme, &tiles, &t), Some(TileCoord::new(1, 1, 0)));
    assert_eq!(find_tile_to_north(&scheme, &tiles, &t), Some(TileCoord::new(0, 0, 0)));
    assert_eq!(find_tile_to_south(&scheme, &tiles, &t), Some(TileCoord::new(0, 2, 0)));
}

#[test]
fn can_get_adjacent_tiles_wrapping_around_antimeridian() {
    let scheme = TilingSchemeDescriptor::new(2, 1);
    let tiles = create_level_zero_tiles(&scheme);

    // X 方向 2 个瓦片时：tile(0,0) 西 → tile(1,0)，东 → tile(1,0)
    let t = TileCoord::new(0, 0, 0);
    assert_eq!(find_tile_to_west(&scheme, &tiles, &t), Some(TileCoord::new(1, 0, 0)));
    assert_eq!(find_tile_to_east(&scheme, &tiles, &t), Some(TileCoord::new(1, 0, 0)));

    let t = TileCoord::new(1, 0, 0);
    assert_eq!(find_tile_to_west(&scheme, &tiles, &t), Some(TileCoord::new(0, 0, 0)));
    assert_eq!(find_tile_to_east(&scheme, &tiles, &t), Some(TileCoord::new(0, 0, 0)));
}

#[test]
fn returns_none_north_of_north_pole_south_of_south_pole() {
    let scheme = TilingSchemeDescriptor::new(2, 1);
    let tiles = create_level_zero_tiles(&scheme);

    let t = TileCoord::new(0, 0, 0);
    assert_eq!(find_tile_to_north(&scheme, &tiles, &t), None);
    assert_eq!(find_tile_to_south(&scheme, &tiles, &t), None);
}

// ─── 子瓦片邻接（共享父瓦片） ──────────────────────────────

#[test]
fn can_get_tiles_around_a_tile_sharing_common_parent() {
    let scheme = TilingSchemeDescriptor::new(2, 1);
    let tiles = create_level_zero_tiles(&scheme);

    // tile(0,0,0) 的子瓦片：NW=(0,0,1), NE=(1,0,1), SW=(0,1,1), SE=(1,1,1)
    let nw = TileCoord::new(0, 0, 1);
    let ne = TileCoord::new(1, 0, 1);
    let sw = TileCoord::new(0, 1, 1);
    let se = TileCoord::new(1, 1, 1);

    // NW 的东邻是 NE（同一父瓦片）
    assert_eq!(find_tile_to_east(&scheme, &tiles, &nw), Some(ne));
    // NW 的南邻是 SW（同一父瓦片）
    assert_eq!(find_tile_to_south(&scheme, &tiles, &nw), Some(sw));
    // NE 的西邻是 NW（同一父瓦片）
    assert_eq!(find_tile_to_west(&scheme, &tiles, &ne), Some(nw));
    // NE 的南邻是 SE（同一父瓦片）
    assert_eq!(find_tile_to_south(&scheme, &tiles, &ne), Some(se));
    // SW 的北邻是 NW（同一父瓦片）
    assert_eq!(find_tile_to_north(&scheme, &tiles, &sw), Some(nw));
    // SW 的东邻是 SE（同一父瓦片）
    assert_eq!(find_tile_to_east(&scheme, &tiles, &sw), Some(se));
    // SE 的北邻是 NE（同一父瓦片）
    assert_eq!(find_tile_to_north(&scheme, &tiles, &se), Some(ne));
    // SE 的西邻是 SW（同一父瓦片）
    assert_eq!(find_tile_to_west(&scheme, &tiles, &se), Some(sw));
}

// ─── 子瓦片邻接（不同父瓦片） ──────────────────────────

#[test]
fn can_get_tiles_around_a_tile_not_sharing_common_parent() {
    let scheme = TilingSchemeDescriptor::new(2, 1);
    let tiles = create_level_zero_tiles(&scheme);

    // Tile(0,0,0) 的子瓦片：NW=(0,0,1), NE=(1,0,1), SW=(0,1,1), SE=(1,1,1)
    // Tile(1,0,0) 的子瓦片：NW=(2,0,1), NE=(3,0,1), SW=(2,1,1), SE=(3,1,1)

    // tile(0,0,0) 的 NE 子瓦片 = (1,0,1)。东邻应为 tile(1,0,0) 的 NW 子瓦片 = (2,0,1)
    let ne_of_first = TileCoord::new(1, 0, 1);
    assert_eq!(
        find_tile_to_east(&scheme, &tiles, &ne_of_first),
        Some(TileCoord::new(2, 0, 1))
    );

    // tile(0,0,0) 的 SE 子瓦片 = (1,1,1)。东邻应为 tile(1,0,0) 的 SW 子瓦片 = (2,1,1)
    let se_of_first = TileCoord::new(1, 1, 1);
    assert_eq!(
        find_tile_to_east(&scheme, &tiles, &se_of_first),
        Some(TileCoord::new(2, 1, 1))
    );

    // tile(1,0,0) 的 NW 子瓦片 = (2,0,1)。西邻应为 tile(0,0,0) 的 NE 子瓦片 = (1,0,1)
    let nw_of_second = TileCoord::new(2, 0, 1);
    assert_eq!(
        find_tile_to_west(&scheme, &tiles, &nw_of_second),
        Some(TileCoord::new(1, 0, 1))
    );

    // tile(1,0,0) 的 SW 子瓦片 = (2,1,1)。西邻应为 tile(0,0,0) 的 SE 子瓦片 = (1,1,1)
    let sw_of_second = TileCoord::new(2, 1, 1);
    assert_eq!(
        find_tile_to_west(&scheme, &tiles, &sw_of_second),
        Some(TileCoord::new(1, 1, 1))
    );
}

// ─── 深层嵌套（层级 2） ─────────────────────────────────────────────────

#[test]
fn adjacency_works_at_deeper_levels() {
    let scheme = TilingSchemeDescriptor::new(2, 1);
    let tiles = create_level_zero_tiles(&scheme);

    // 层级 2：(0,0,1) 的子瓦片是 (0,0,2), (1,0,2), (0,1,2), (1,1,2)
    let nw_l2 = TileCoord::new(0, 0, 2);
    let ne_l2 = TileCoord::new(1, 0, 2);

    // 同一父瓦片邻接
    assert_eq!(find_tile_to_east(&scheme, &tiles, &nw_l2), Some(ne_l2));
    assert_eq!(find_tile_to_west(&scheme, &tiles, &ne_l2), Some(nw_l2));

    // 跨父瓦片：(0,0,1) 的 NE 是 (1,0,1)。它的 NW 子瓦片是 (2,0,2)。
    // 所以 (1,0,2) [(0,0,1) 的 NE 子瓦片] 的东邻应先上升到父瓦片 (0,0,1)，
    // 找到父瓦片的东邻 = (1,0,1)，再取其 NW 子瓦片 = (2,0,2)
    assert_eq!(
        find_tile_to_east(&scheme, &tiles, &ne_l2),
        Some(TileCoord::new(2, 0, 2))
    );
}

// ─── Geographic 镶嵌方案 (2x1) ─────────────────────────────────────────

#[test]
fn geographic_scheme_level_zero() {
    let scheme = TilingSchemeDescriptor::geographic();
    let tiles = create_level_zero_tiles(&scheme);
    assert_eq!(tiles.len(), 2);
    assert_eq!(tiles[0], TileCoord::new(0, 0, 0));
    assert_eq!(tiles[1], TileCoord::new(1, 0, 0));
}

#[test]
fn web_mercator_scheme_level_zero() {
    let scheme = TilingSchemeDescriptor::web_mercator();
    let tiles = create_level_zero_tiles(&scheme);
    assert_eq!(tiles.len(), 1);
    assert_eq!(tiles[0], TileCoord::new(0, 0, 0));
}
