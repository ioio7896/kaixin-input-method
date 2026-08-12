use super::entities::*;
use super::*;

const MIN_SEGMENT_LENGTH: f32 = 12.5;
const MAX_LOCAL_LENGTH_RATIO: f32 = 1.1;
const MAX_RUNNING_LENGTH_RATIO: f32 = 1.09;

struct Rect {
    pub top: f32,
    pub bottom: f32,
    pub left: f32,
    pub right: f32,
}

pub struct AnalyzedCharacter<'a> {
    pub analyzed_strokes: Vec<AnalyzedStroke<'a>>,
    pub sub_stroke_count: usize,
}

impl<'a> AnalyzedCharacter<'a> {
    pub fn from_strokes(strokes: &Vec<Stroke>) -> AnalyzedCharacter {
        let bounding_rect = get_bounding_rect(strokes);
        let analyzed_strokes: Vec<AnalyzedStroke> = build_analyzed_strokes(strokes, &bounding_rect);
        let mut sub_stroke_count: usize = 0;
        for i in 0..analyzed_strokes.len() {
            sub_stroke_count += analyzed_strokes[i].sub_strokes.len();
        }
        AnalyzedCharacter {
            analyzed_strokes,
            sub_stroke_count,
        }
    }

    pub fn get_analyzed_strokes(&self) -> Vec<SubStroke> {
        let mut res: Vec<SubStroke> = Vec::with_capacity(self.sub_stroke_count);
        for i in 0..self.analyzed_strokes.len() {
            for j in 0..self.analyzed_strokes[i].sub_strokes.len() {
                res.push(self.analyzed_strokes[i].sub_strokes[j]);
            }
        }
        res
    }
}

// Gets distance between two points
fn dist(a: Point, b: Point) -> f32 {
    let dx = (a.x as f32) - (b.x as f32);
    let dy = (a.y as f32) - (b.y as f32);
    (dx * dx + dy * dy).sqrt()
}

// Gets normalized distance between two points
// Normalized based on bounding rectangle
fn norm_dist(a: Point, b: Point, bounding_rect: &Rect) -> f32 {
    let width = bounding_rect.right - bounding_rect.left;
    let height = bounding_rect.bottom - bounding_rect.top;
    // normalizer is a diagonal along a square with sides of size the larger dimension of the bounding box
    let dim_squared;
    if width > height {
        dim_squared = width * width;
    } else {
        dim_squared = height * height;
    }
    let normalizer = (dim_squared + dim_squared).sqrt();
    let dist_norm = dist(a, b) / normalizer;
    // Cap at 1 (...why is this needed??)
    f32::min(dist_norm, 1f32)
}

// Gets direction, in radians, from point a to b
// 0 is to the right, PI / 2 is up, etc.
fn dir(a: Point, b: Point) -> f32 {
    let dx = (a.x as f32) - (b.x as f32);
    let dy = (a.y as f32) - (b.y as f32);
    let dir = dy.atan2(dx);
    std::f32::consts::PI - dir
}

fn get_norm_center(a: Point, b: Point, bounding_rect: &Rect) -> (f32, f32) {
    let mut x = ((a.x as f32) + (b.x as f32)) / 2f32;
    let mut y = ((a.y as f32) + (b.y as f32)) / 2f32;
    let side;
    // Bounding rect is landscape
    if bounding_rect.right - bounding_rect.left > bounding_rect.bottom - bounding_rect.top {
        side = bounding_rect.right - bounding_rect.left;
        let height = bounding_rect.bottom - bounding_rect.top;
        x -= bounding_rect.left;
        y = y - bounding_rect.top + (side - height) / 2f32;
    }
    // Portrait
    else {
        side = bounding_rect.bottom - bounding_rect.top;
        let width = bounding_rect.right - bounding_rect.left;
        x = x - bounding_rect.left + (side - width) / 2f32;
        y -= bounding_rect.top;
    }
    (x / side, y / side)
}

// Calculates array with indexes of pivot points in raw stroke
fn get_pivot_indexes(stroke: &Stroke) -> Vec<usize> {
    let points = &stroke.points;

    // One item for each point: true if it's a pivot
    let mut markers: Vec<bool> = Vec::with_capacity(points.len());
    for _ in 0..points.len() {
        markers.push(false);
    }

    // Cycle variables
    let mut prev_pt_ix = 0;
    let mut first_pt_ix = 0;
    let mut pivot_pt_ix = 1;

    // The first point of a Stroke is always a pivot point.
    markers[0] = true;

    // localLength keeps track of the immediate distance between the latest three points.
    // We can use localLength to find an abrupt change in substrokes, such as at a corner.
    // We do this by checking localLength against the distance between the first and last
    // of the three points. If localLength is more than a certain amount longer than the
    // length between the first and last point, then there must have been a corner of some kind.
    let mut local_length = dist(points[first_pt_ix], points[pivot_pt_ix]);

    // runningLength keeps track of the length between the start of the current SubStroke
    // and the point we are currently examining.  If the runningLength becomes a certain
    // amount longer than the straight distance between the first point and the current
    // point, then there is a new SubStroke.  This accounts for a more gradual change
    // from one SubStroke segment to another, such as at a longish curve.
    let mut running_length = local_length;

    // Cycle through rest of stroke points.
    let mut i = 2;
    while i < points.len() {
        let next_point = points[i];

        // pivotPoint is the point we're currently examining to see if it's a pivot.
        // We get the distance between this point and the next point and add it
        // to the length sums we're using.
        let pivot_length = dist(points[pivot_pt_ix], next_point);
        local_length += pivot_length;
        running_length += pivot_length;

        // Check the lengths against the ratios.  If the lengths are a certain among
        // longer than a straight line between the first and last point, then we
        // mark the point as a pivot.
        let dist_from_previous = dist(points[prev_pt_ix], next_point);
        let dist_from_first = dist(points[first_pt_ix], next_point);
        if local_length > MAX_LOCAL_LENGTH_RATIO * dist_from_previous
            || running_length > MAX_RUNNING_LENGTH_RATIO * dist_from_first
        {
            // If the previous point was a pivot and was very close to this point,
            // which we are about to mark as a pivot, then unmark the previous point as a pivot.
            if markers[prev_pt_ix]
                && dist(points[prev_pt_ix], points[pivot_pt_ix]) < MIN_SEGMENT_LENGTH
            {
                markers[prev_pt_ix] = false;
            }
            markers[pivot_pt_ix] = true;
            running_length = pivot_length;
            first_pt_ix = pivot_pt_ix;
        }
        local_length = pivot_length;
        prev_pt_ix = pivot_pt_ix;
        pivot_pt_ix = i;

        i += 1;
    }

    // last point (currently referenced by pivotPoint) has to be a pivot
    markers[pivot_pt_ix] = true;
    // Point before the final point may need to be handled specially.
    // Often mouse action will produce an unintended small segment at the end.
    // We'll want to unmark the previous point if it's also a pivot and very close to the lat point.
    // However if the previous point is the first point of the stroke, then don't unmark it, because
    // then we'd only have one pivot.
    if markers[prev_pt_ix]
        && dist(points[prev_pt_ix], points[pivot_pt_ix]) < MIN_SEGMENT_LENGTH
        && prev_pt_ix != 0
    {
        markers[prev_pt_ix] = false;
    }

    // Return result in the form of an index array: includes indexes where marker is true
    let mut marker_count = 0;
    for x in &markers {
        if *x {
            marker_count += 1;
        }
    }
    let mut res: Vec<usize> = Vec::with_capacity(marker_count);
    for ix in 0..markers.len() {
        if markers[ix] {
            res.push(ix);
        }
    }
    res
}

// Builds array of substrokes from stroke's points, pivots, and character's bounding rectangle
fn build_sub_strokes(
    stroke: &Stroke,
    pivot_indexes: &Vec<usize>,
    bounding_rect: &Rect,
) -> Vec<SubStroke> {
    let mut res: Vec<SubStroke> = Vec::new();
    let mut prev_ix: usize = 0;
    for i in 0..pivot_indexes.len() {
        let ix = pivot_indexes[i];
        if ix == prev_ix {
            continue;
        }
        let mut direction = dir(stroke.points[prev_ix], stroke.points[ix]);
        direction = (direction * 256f32 / std::f32::consts::PI / 2f32).round();
        if direction >= 256f32 {
            direction = 0f32;
        }
        let mut norm_length = norm_dist(stroke.points[prev_ix], stroke.points[ix], bounding_rect);
        norm_length = (norm_length * 255f32).round();
        let center = get_norm_center(stroke.points[prev_ix], stroke.points[ix], bounding_rect);
        res.push(SubStroke {
            direction,
            length: norm_length,
            center_x: (center.0 * 15f32).round(),
            center_y: (center.1 * 15f32).round(),
        });
        prev_ix = ix;
    }
    res
}

// Analyze raw input, store result in _analyzedStrokes member.
fn build_analyzed_strokes<'a>(
    strokes: &'a Vec<Stroke>,
    bounding_rect: &Rect,
) -> Vec<AnalyzedStroke<'a>> {
    let mut res: Vec<AnalyzedStroke> = Vec::new();
    // Process each stroke
    for stroke in strokes {
        // Identify pivot points
        let pivot_indexes = get_pivot_indexes(stroke);
        // Abstract away substrokes
        let sub_strokes = build_sub_strokes(stroke, &pivot_indexes, bounding_rect);
        // Store all this
        res.push(AnalyzedStroke {
            points: &stroke.points,
            pivot_indexes,
            sub_strokes,
        });
    }
    res
}

fn get_bounding_rect(strokes: &Vec<Stroke>) -> Rect {
    let mut res = Rect {
        top: std::f32::MAX,
        bottom: std::f32::MIN,
        left: std::f32::MAX,
        right: std::f32::MIN,
    };
    for stroke in strokes {
        for pt in &stroke.points {
            if (pt.x as f32) < res.left {
                res.left = pt.x as f32;
            }
            if (pt.x as f32) > res.right {
                res.right = pt.x as f32;
            }
            if (pt.y as f32) < res.top {
                res.top = pt.y as f32;
            }
            if (pt.y as f32) > res.bottom {
                res.bottom = pt.y as f32;
            }
        }
    }
    if res.top > 255f32 {
        res.top = 0f32;
    }
    if res.bottom < 0f32 {
        res.bottom = 255f32;
    }
    if res.left > 255f32 {
        res.left = 0f32;
    }
    if res.right < 0f32 {
        res.right = 255f32;
    }
    res
}
