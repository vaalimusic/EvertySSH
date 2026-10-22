//! Small deterministic measure/layout primitives. Widgets consume the returned
//! rectangles for both painting and interaction; this module knows no app logic.

use crate::{Constraints, Insets, LogicalPx, LogicalRect, LogicalSize};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct LayoutId(pub u64);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum MainAxis {
    #[default]
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Alignment {
    #[default]
    Start,
    Center,
    End,
    Stretch,
}

#[derive(Clone, Debug)]
pub struct FlexChild {
    pub node: LayoutNode,
    /// Zero keeps the intrinsic main-axis size; positive values share free space.
    pub flex: f64,
}

impl FlexChild {
    pub fn fixed(node: LayoutNode) -> Self {
        Self { node, flex: 0.0 }
    }

    pub fn flexible(node: LayoutNode, flex: f64) -> Self {
        Self {
            node,
            flex: if flex.is_finite() { flex.max(0.0) } else { 0.0 },
        }
    }
}

#[derive(Clone, Debug)]
pub enum LayoutKind {
    /// A leaf with an intrinsic preferred size.
    Intrinsic(LogicalSize),
    /// A leaf that takes all available space, subject to its node constraints.
    Spacer,
    Flex {
        axis: MainAxis,
        children: Vec<FlexChild>,
        gap: LogicalPx,
        padding: Insets,
        cross_alignment: Alignment,
    },
    Stack(Vec<LayoutNode>),
    Padding {
        padding: Insets,
        child: Box<LayoutNode>,
    },
    Align {
        horizontal: Alignment,
        vertical: Alignment,
        child: Box<LayoutNode>,
    },
}

#[derive(Clone, Debug)]
pub struct LayoutNode {
    pub id: LayoutId,
    pub kind: LayoutKind,
    pub min: LogicalSize,
    pub max: Option<LogicalSize>,
}

impl LayoutNode {
    pub fn intrinsic(id: LayoutId, size: LogicalSize) -> Self {
        Self::new(id, LayoutKind::Intrinsic(sanitized_size(size)))
    }

    pub fn spacer(id: LayoutId) -> Self {
        Self::new(id, LayoutKind::Spacer)
    }

    pub fn row(id: LayoutId, children: Vec<FlexChild>, gap: f64) -> Self {
        Self::flex(id, MainAxis::Horizontal, children, gap, Insets::ZERO)
    }

    pub fn column(id: LayoutId, children: Vec<FlexChild>, gap: f64) -> Self {
        Self::flex(id, MainAxis::Vertical, children, gap, Insets::ZERO)
    }

    pub fn flex(
        id: LayoutId,
        axis: MainAxis,
        children: Vec<FlexChild>,
        gap: f64,
        padding: Insets,
    ) -> Self {
        Self::new(
            id,
            LayoutKind::Flex {
                axis,
                children,
                gap: LogicalPx(non_negative(gap)),
                padding,
                cross_alignment: Alignment::Stretch,
            },
        )
    }

    pub fn stack(id: LayoutId, children: Vec<LayoutNode>) -> Self {
        Self::new(id, LayoutKind::Stack(children))
    }

    pub fn padding(id: LayoutId, padding: Insets, child: LayoutNode) -> Self {
        Self::new(
            id,
            LayoutKind::Padding {
                padding,
                child: Box::new(child),
            },
        )
    }

    pub fn align(
        id: LayoutId,
        horizontal: Alignment,
        vertical: Alignment,
        child: LayoutNode,
    ) -> Self {
        Self::new(
            id,
            LayoutKind::Align {
                horizontal,
                vertical,
                child: Box::new(child),
            },
        )
    }

    pub fn with_min(mut self, min: LogicalSize) -> Self {
        self.min = sanitized_size(min);
        self
    }

    pub fn with_max(mut self, max: LogicalSize) -> Self {
        self.max = Some(LogicalSize::new(
            non_negative_or_infinite(max.width.0),
            non_negative_or_infinite(max.height.0),
        ));
        self
    }

    fn new(id: LayoutId, kind: LayoutKind) -> Self {
        Self {
            id,
            kind,
            min: LogicalSize::ZERO,
            max: None,
        }
    }

    pub fn measure(&self, constraints: Constraints) -> LogicalSize {
        let raw = match &self.kind {
            LayoutKind::Intrinsic(size) => *size,
            LayoutKind::Spacer => constraints.max.unwrap_or(LogicalSize::ZERO),
            LayoutKind::Stack(children) => children
                .iter()
                .map(|child| child.measure(Constraints::UNBOUNDED))
                .fold(LogicalSize::ZERO, max_size),
            LayoutKind::Padding { padding, child } => add_insets(
                child.measure(inner_constraints(constraints, *padding)),
                *padding,
            ),
            LayoutKind::Align { child, .. } => child.measure(constraints),
            LayoutKind::Flex {
                axis,
                children,
                gap,
                padding,
                ..
            } => {
                let sizes: Vec<_> = children
                    .iter()
                    .map(|child| child.node.measure(Constraints::UNBOUNDED))
                    .collect();
                let gap_total = gap.0 * children.len().saturating_sub(1) as f64;
                let combined = match axis {
                    MainAxis::Horizontal => LogicalSize::new(
                        sizes.iter().map(|s| s.width.0).sum::<f64>() + gap_total,
                        sizes.iter().map(|s| s.height.0).fold(0.0, f64::max),
                    ),
                    MainAxis::Vertical => LogicalSize::new(
                        sizes.iter().map(|s| s.width.0).fold(0.0, f64::max),
                        sizes.iter().map(|s| s.height.0).sum::<f64>() + gap_total,
                    ),
                };
                add_insets(combined, *padding)
            }
        };
        self.constrain(constraints.constrain(raw))
    }

    fn constrain(&self, size: LogicalSize) -> LogicalSize {
        let max = self
            .max
            .unwrap_or(LogicalSize::new(f64::INFINITY, f64::INFINITY));
        LogicalSize::new(
            size.width
                .0
                .max(self.min.width.0)
                .min(max.width.0.max(self.min.width.0)),
            size.height
                .0
                .max(self.min.height.0)
                .min(max.height.0.max(self.min.height.0)),
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayoutBox {
    pub id: LayoutId,
    pub rect: LogicalRect,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LayoutResult {
    boxes: Vec<LayoutBox>,
}

impl LayoutResult {
    /// Linear lookup: fine for the handful of ids a widget tree queries per
    /// frame; iterate with [`LayoutResult::iter`] when visiting every box.
    pub fn get(&self, id: LayoutId) -> Option<LogicalRect> {
        self.boxes
            .iter()
            .find(|item| item.id == id)
            .map(|item| item.rect)
    }

    pub fn iter(&self) -> impl Iterator<Item = &LayoutBox> {
        self.boxes.iter()
    }

    pub fn hit_test(&self, point: crate::LogicalPoint) -> Option<LayoutId> {
        self.boxes
            .iter()
            .rev()
            .find(|item| item.rect.contains(point))
            .map(|item| item.id)
    }
}

pub fn layout(root: &LayoutNode, bounds: LogicalRect) -> LayoutResult {
    let mut result = LayoutResult::default();
    place(root, bounds, &mut result);
    result
}

fn place(node: &LayoutNode, bounds: LogicalRect, output: &mut LayoutResult) {
    let rect = LogicalRect::new(bounds.origin, node.constrain(bounds.size));
    output.boxes.push(LayoutBox { id: node.id, rect });
    match &node.kind {
        LayoutKind::Intrinsic(_) | LayoutKind::Spacer => {}
        LayoutKind::Stack(children) => {
            for child in children {
                place(child, rect, output);
            }
        }
        LayoutKind::Padding { padding, child } => {
            place(child, rect.inset(*padding), output);
        }
        LayoutKind::Align {
            horizontal,
            vertical,
            child,
        } => {
            let natural = child.measure(Constraints::UNBOUNDED);
            let child_rect = aligned_rect(rect, natural, *horizontal, *vertical);
            place(child, child_rect, output);
        }
        LayoutKind::Flex {
            axis,
            children,
            gap,
            padding,
            cross_alignment,
        } => {
            let inner = rect.inset(*padding);
            place_flex(*axis, children, *gap, *cross_alignment, inner, output);
        }
    }
}

fn place_flex(
    axis: MainAxis,
    children: &[FlexChild],
    gap: LogicalPx,
    cross_alignment: Alignment,
    bounds: LogicalRect,
    output: &mut LayoutResult,
) {
    if children.is_empty() {
        return;
    }
    let main_available = match axis {
        MainAxis::Horizontal => bounds.size.width.0,
        MainAxis::Vertical => bounds.size.height.0,
    };
    let gaps = gap.0 * children.len().saturating_sub(1) as f64;
    let mut desired: Vec<f64> = children
        .iter()
        .map(|child| {
            if child.flex > 0.0 {
                0.0
            } else {
                main_size(child.node.measure(Constraints::UNBOUNDED), axis)
            }
        })
        .collect();
    let fixed: f64 = desired.iter().sum();
    resolve_flexible(
        children,
        axis,
        (main_available - gaps - fixed).max(0.0),
        &mut desired,
    );

    let mut cursor = main_start(bounds, axis);
    for (index, (child, desired_main)) in children.iter().zip(desired).enumerate() {
        let available_cross = cross_size(bounds.size, axis);
        let natural = child.node.measure(Constraints::UNBOUNDED);
        let natural_cross = cross_size(natural, axis);
        let child_cross = match cross_alignment {
            Alignment::Stretch => available_cross,
            _ => natural_cross.min(available_cross),
        };
        let cross_offset = match cross_alignment {
            Alignment::Start | Alignment::Stretch => 0.0,
            Alignment::Center => (available_cross - child_cross) / 2.0,
            Alignment::End => available_cross - child_cross,
        };
        let child_rect = match axis {
            MainAxis::Horizontal => LogicalRect::from_xywh(
                cursor,
                bounds.top() + cross_offset,
                desired_main,
                child_cross,
            ),
            MainAxis::Vertical => LogicalRect::from_xywh(
                bounds.left() + cross_offset,
                cursor,
                child_cross,
                desired_main,
            ),
        };
        place(&child.node, child_rect, output);
        cursor += desired_main;
        if index + 1 < children.len() {
            cursor += gap.0;
        }
    }
}

/// Share `free` main-axis space among the flexible children by weight while
/// honouring each child's own min/max: a child whose share violates a limit is
/// frozen at that limit and the remainder is redistributed (CSS-flex style), so
/// siblings never overlap and no child is given more or less than it can take.
fn resolve_flexible(children: &[FlexChild], axis: MainAxis, free: f64, desired: &mut [f64]) {
    if children.iter().all(|child| child.flex <= 0.0) {
        return;
    }
    let limits = |child: &FlexChild| {
        let node = &child.node;
        let (min, max) = match axis {
            MainAxis::Horizontal => (node.min.width.0, node.max.map(|m| m.width.0)),
            MainAxis::Vertical => (node.min.height.0, node.max.map(|m| m.height.0)),
        };
        (min, max.map_or(f64::INFINITY, |max| max.max(min)))
    };
    let mut frozen: Vec<bool> = children.iter().map(|child| child.flex <= 0.0).collect();
    let mut remaining = free;
    // Each pass freezes at least one child or terminates.
    for _ in 0..=children.len() {
        let total: f64 = children
            .iter()
            .zip(&frozen)
            .filter(|(_, frozen)| !**frozen)
            .map(|(child, _)| child.flex)
            .sum();
        if total <= 0.0 {
            break;
        }
        let mut violated = false;
        for (index, child) in children.iter().enumerate() {
            if frozen[index] {
                continue;
            }
            let share = remaining.max(0.0) * child.flex / total;
            let (min, max) = limits(child);
            let clamped = share.clamp(min, max);
            desired[index] = clamped;
            if clamped != share {
                frozen[index] = true;
                violated = true;
            }
        }
        if !violated {
            break;
        }
        remaining = free
            - children
                .iter()
                .enumerate()
                .filter(|(index, child)| frozen[*index] && child.flex > 0.0)
                .map(|(index, _)| desired[index])
                .sum::<f64>();
    }
}

fn sanitized_size(size: LogicalSize) -> LogicalSize {
    LogicalSize::new(non_negative(size.width.0), non_negative(size.height.0))
}

fn non_negative_or_infinite(value: f64) -> f64 {
    if value.is_nan() {
        0.0
    } else {
        value.max(0.0)
    }
}

fn aligned_rect(
    bounds: LogicalRect,
    natural: LogicalSize,
    horizontal: Alignment,
    vertical: Alignment,
) -> LogicalRect {
    let width = if horizontal == Alignment::Stretch {
        bounds.size.width.0
    } else {
        natural.width.0.min(bounds.size.width.0)
    };
    let height = if vertical == Alignment::Stretch {
        bounds.size.height.0
    } else {
        natural.height.0.min(bounds.size.height.0)
    };
    let x = match horizontal {
        Alignment::Start | Alignment::Stretch => bounds.left(),
        Alignment::Center => bounds.left() + (bounds.size.width.0 - width) / 2.0,
        Alignment::End => bounds.right() - width,
    };
    let y = match vertical {
        Alignment::Start | Alignment::Stretch => bounds.top(),
        Alignment::Center => bounds.top() + (bounds.size.height.0 - height) / 2.0,
        Alignment::End => bounds.bottom() - height,
    };
    LogicalRect::from_xywh(x, y, width, height)
}

fn inner_constraints(constraints: Constraints, insets: Insets) -> Constraints {
    let insets = sanitized_insets(insets);
    Constraints::new(
        LogicalSize::ZERO,
        constraints.max.map(|size| {
            LogicalSize::new(
                (size.width.0 - insets.left.0 - insets.right.0).max(0.0),
                (size.height.0 - insets.top.0 - insets.bottom.0).max(0.0),
            )
        }),
    )
}

fn add_insets(size: LogicalSize, insets: Insets) -> LogicalSize {
    let insets = sanitized_insets(insets);
    LogicalSize::new(
        size.width.0 + insets.left.0 + insets.right.0,
        size.height.0 + insets.top.0 + insets.bottom.0,
    )
}

/// Non-finite insets count as zero, matching `LogicalRect::inset`, so the
/// measured size and the placed rectangle agree.
fn sanitized_insets(insets: Insets) -> Insets {
    let finite = |value: LogicalPx| LogicalPx(if value.0.is_finite() { value.0 } else { 0.0 });
    Insets {
        left: finite(insets.left),
        top: finite(insets.top),
        right: finite(insets.right),
        bottom: finite(insets.bottom),
    }
}

fn max_size(a: LogicalSize, b: LogicalSize) -> LogicalSize {
    LogicalSize::new(a.width.0.max(b.width.0), a.height.0.max(b.height.0))
}

fn main_size(size: LogicalSize, axis: MainAxis) -> f64 {
    match axis {
        MainAxis::Horizontal => size.width.0,
        MainAxis::Vertical => size.height.0,
    }
}

fn cross_size(size: LogicalSize, axis: MainAxis) -> f64 {
    match axis {
        MainAxis::Horizontal => size.height.0,
        MainAxis::Vertical => size.width.0,
    }
}

fn main_start(bounds: LogicalRect, axis: MainAxis) -> f64 {
    match axis {
        MainAxis::Horizontal => bounds.left(),
        MainAxis::Vertical => bounds.top(),
    }
}

fn non_negative(value: f64) -> f64 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LogicalPoint;

    #[test]
    fn flex_layout_measures_and_distributes_remaining_space() {
        let tree = LayoutNode::row(
            LayoutId(1),
            vec![
                FlexChild::fixed(LayoutNode::intrinsic(
                    LayoutId(2),
                    LogicalSize::new(20.0, 10.0),
                )),
                FlexChild::flexible(LayoutNode::spacer(LayoutId(3)), 1.0),
                FlexChild::flexible(
                    LayoutNode::intrinsic(LayoutId(4), LogicalSize::new(5.0, 10.0)),
                    2.0,
                ),
            ],
            5.0,
        );
        let rect = LogicalRect::from_xywh(10.0, 20.0, 100.0, 40.0);
        let layout = layout(&tree, rect);
        assert_eq!(
            layout.get(LayoutId(2)).unwrap(),
            LogicalRect::from_xywh(10.0, 20.0, 20.0, 40.0)
        );
        assert_eq!(
            layout.get(LayoutId(3)).unwrap().size.width.0,
            23.333333333333332
        );
        assert!((layout.get(LayoutId(4)).unwrap().size.width.0 - 46.666666666666664).abs() < 1e-9);
        assert_eq!(
            tree.measure(Constraints::UNBOUNDED),
            LogicalSize::new(35.0, 10.0)
        );
    }

    #[test]
    fn nested_padding_alignment_and_hit_test_share_layout_geometry() {
        let leaf = LayoutNode::intrinsic(LayoutId(3), LogicalSize::new(20.0, 10.0));
        let tree = LayoutNode::padding(
            LayoutId(1),
            Insets::all(4.0),
            LayoutNode::align(LayoutId(2), Alignment::Center, Alignment::Center, leaf),
        );
        let layout = layout(&tree, LogicalRect::from_xywh(0.0, 0.0, 60.0, 40.0));
        assert_eq!(
            layout.get(LayoutId(2)).unwrap(),
            LogicalRect::from_xywh(4.0, 4.0, 52.0, 32.0)
        );
        assert_eq!(
            layout.get(LayoutId(3)).unwrap(),
            LogicalRect::from_xywh(20.0, 15.0, 20.0, 10.0)
        );
        assert_eq!(
            layout.hit_test(LogicalPoint::new(21.0, 16.0)),
            Some(LayoutId(3))
        );
    }

    #[test]
    fn stack_hit_testing_uses_last_painted_child() {
        let stack = LayoutNode::stack(
            LayoutId(1),
            vec![
                LayoutNode::intrinsic(LayoutId(2), LogicalSize::new(20.0, 20.0)),
                LayoutNode::intrinsic(LayoutId(3), LogicalSize::new(10.0, 10.0)),
            ],
        );
        let result = layout(&stack, LogicalRect::from_xywh(0.0, 0.0, 50.0, 50.0));
        assert_eq!(
            result.hit_test(LogicalPoint::new(5.0, 5.0)),
            Some(LayoutId(3))
        );
    }

    fn spacer(id: u64) -> LayoutNode {
        LayoutNode::spacer(LayoutId(id))
    }

    #[test]
    fn flexible_children_respect_min_and_max_and_never_overlap() {
        // Equal weights in 100px, but the first needs at least 80px.
        let tree = LayoutNode::row(
            LayoutId(1),
            vec![
                FlexChild::flexible(spacer(2).with_min(LogicalSize::new(80.0, 0.0)), 1.0),
                FlexChild::flexible(spacer(3), 1.0),
            ],
            0.0,
        );
        let result = layout(&tree, LogicalRect::from_xywh(0.0, 0.0, 100.0, 10.0));
        assert_eq!(result.get(LayoutId(2)).unwrap().size.width.0, 80.0);
        let second = result.get(LayoutId(3)).unwrap();
        assert_eq!((second.left(), second.size.width.0), (80.0, 20.0));

        // A capped child frees its unused share for the others.
        let tree = LayoutNode::row(
            LayoutId(1),
            vec![
                FlexChild::flexible(spacer(2).with_max(LogicalSize::new(10.0, 99.0)), 1.0),
                FlexChild::flexible(spacer(3), 1.0),
            ],
            4.0,
        );
        let result = layout(&tree, LogicalRect::from_xywh(0.0, 0.0, 100.0, 10.0));
        assert_eq!(result.get(LayoutId(2)).unwrap().size.width.0, 10.0);
        let second = result.get(LayoutId(3)).unwrap();
        assert_eq!((second.left(), second.size.width.0), (14.0, 86.0));
    }

    #[test]
    fn fixed_content_larger_than_the_container_overflows_in_order_without_negative_sizes() {
        let tree = LayoutNode::row(
            LayoutId(1),
            vec![
                FlexChild::fixed(LayoutNode::intrinsic(
                    LayoutId(2),
                    LogicalSize::new(80.0, 10.0),
                )),
                FlexChild::flexible(spacer(3), 1.0),
                FlexChild::fixed(LayoutNode::intrinsic(
                    LayoutId(4),
                    LogicalSize::new(80.0, 10.0),
                )),
            ],
            0.0,
        );
        let result = layout(&tree, LogicalRect::from_xywh(0.0, 0.0, 100.0, 10.0));
        assert_eq!(result.get(LayoutId(3)).unwrap().size.width.0, 0.0);
        assert_eq!(result.get(LayoutId(2)).unwrap().right(), 80.0);
        assert_eq!(result.get(LayoutId(4)).unwrap().left(), 80.0);
    }

    #[test]
    fn degenerate_inputs_never_produce_nan_or_negative_geometry() {
        let hostile = [
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
            -5.0,
            0.0,
            1e-9,
            3.0,
            97.5,
            1e7,
        ];
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        let mut pick = |bound: usize| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state % bound as u64) as usize
        };
        for case in 0..400u64 {
            let mut next_id = 10;
            let mut leaf = |pick: &mut dyn FnMut(usize) -> usize| {
                next_id += 1;
                let size = LogicalSize::new(hostile[pick(9)], hostile[pick(9)]);
                let node = if pick(4) == 0 {
                    spacer(next_id)
                } else {
                    LayoutNode::intrinsic(LayoutId(next_id), size)
                };
                let node = if pick(4) == 0 {
                    next_id += 1;
                    LayoutNode::padding(
                        LayoutId(next_id),
                        Insets::symmetric(hostile[pick(9)], hostile[pick(9)]),
                        node,
                    )
                } else {
                    node
                };
                match pick(4) {
                    0 => node.with_min(LogicalSize::new(hostile[pick(9)], hostile[pick(9)])),
                    1 => node.with_max(LogicalSize::new(hostile[pick(9)], hostile[pick(9)])),
                    _ => node,
                }
            };
            let children: Vec<_> = (0..1 + pick(5))
                .map(|_| {
                    let node = leaf(&mut pick);
                    if pick(2) == 0 {
                        FlexChild::flexible(node, hostile[pick(9)])
                    } else {
                        FlexChild::fixed(node)
                    }
                })
                .collect();
            let axis = if pick(2) == 0 {
                MainAxis::Horizontal
            } else {
                MainAxis::Vertical
            };
            let padding = Insets::symmetric(hostile[pick(9)], hostile[pick(9)]);
            let inner = LayoutNode::flex(LayoutId(2), axis, children, hostile[pick(9)], padding);
            let tree = LayoutNode::align(
                LayoutId(1),
                [
                    Alignment::Start,
                    Alignment::Center,
                    Alignment::End,
                    Alignment::Stretch,
                ][pick(4)],
                [
                    Alignment::Start,
                    Alignment::Center,
                    Alignment::End,
                    Alignment::Stretch,
                ][pick(4)],
                LayoutNode::padding(LayoutId(3), padding, inner),
            );
            let bounds_extent = [0.0, 1.0, 17.5, 320.0, 4096.0, 1e6];
            let bounds = LogicalRect::from_xywh(
                [0.0, -50.0, 12.5][pick(3)],
                [0.0, 7.0][pick(2)],
                bounds_extent[pick(6)],
                bounds_extent[pick(6)],
            );
            let result = layout(&tree, bounds);
            for item in result.iter() {
                let rect = item.rect;
                for value in [
                    rect.left(),
                    rect.top(),
                    rect.size.width.0,
                    rect.size.height.0,
                ] {
                    assert!(
                        value.is_finite(),
                        "case {case}: non-finite geometry {rect:?} for {:?}",
                        item.id
                    );
                    assert!(!value.is_nan(), "case {case}: NaN in {rect:?}");
                }
                assert!(
                    rect.size.width.0 >= 0.0 && rect.size.height.0 >= 0.0,
                    "case {case}: negative size {rect:?}"
                );
            }
        }
    }
}
