//! Line icons drawn with egui's painter (24×24 design grid, same shapes as the approved mockup).

use egui::{
    Color32, CornerRadius, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, epaint::PathStroke, pos2,
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Icon {
    List,
    Download,
    Clock,
    Check,
    Alert,
    Video,
    Music,
    Image,
    Doc,
    Archive,
    Program,
    Disk,
    File,
    Plus,
    Play,
    Pause,
    Trash,
    PauseCircle,
    Queue,
    Folder,
    Gauge,
    Sun,
    Moon,
    Monitor,
    Link,
    Close,
    Upload,
    Plug,
}

struct Pen<'a> {
    p: &'a Painter,
    r: Rect,
    color: Color32,
    w: f32,
}

impl Pen<'_> {
    fn pt(&self, x: f32, y: f32) -> Pos2 {
        pos2(
            self.r.min.x + x / 24.0 * self.r.width(),
            self.r.min.y + y / 24.0 * self.r.height(),
        )
    }
    fn s(&self, v: f32) -> f32 {
        v / 24.0 * self.r.width()
    }
    fn line(&self, pts: &[(f32, f32)]) {
        let v: Vec<Pos2> = pts.iter().map(|(x, y)| self.pt(*x, *y)).collect();
        self.p
            .add(Shape::line(v, PathStroke::new(self.w, self.color)));
    }
    fn closed(&self, pts: &[(f32, f32)]) {
        let v: Vec<Pos2> = pts.iter().map(|(x, y)| self.pt(*x, *y)).collect();
        self.p
            .add(Shape::closed_line(v, Stroke::new(self.w, self.color)));
    }
    fn fill(&self, pts: &[(f32, f32)]) {
        let v: Vec<Pos2> = pts.iter().map(|(x, y)| self.pt(*x, *y)).collect();
        self.p
            .add(Shape::convex_polygon(v, self.color, Stroke::NONE));
    }
    fn circle(&self, x: f32, y: f32, r: f32) {
        self.p
            .circle_stroke(self.pt(x, y), self.s(r), Stroke::new(self.w, self.color));
    }
    fn dot(&self, x: f32, y: f32, r: f32) {
        self.p.circle_filled(self.pt(x, y), self.s(r), self.color);
    }
    fn rect(&self, x: f32, y: f32, w: f32, h: f32, rad: f32) {
        let r = Rect::from_min_max(self.pt(x, y), self.pt(x + w, y + h));
        self.p.rect_stroke(
            r,
            CornerRadius::same(self.s(rad) as u8),
            Stroke::new(self.w, self.color),
            StrokeKind::Middle,
        );
    }
    fn arc(&self, cx: f32, cy: f32, r: f32, from_deg: f32, to_deg: f32) {
        let n = 20;
        let pts: Vec<(f32, f32)> = (0..=n)
            .map(|i| {
                let a = (from_deg + (to_deg - from_deg) * i as f32 / n as f32).to_radians();
                (cx + r * a.cos(), cy + r * a.sin())
            })
            .collect();
        self.line(&pts);
    }
}

pub fn paint(painter: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let w = (rect.width() / 24.0 * 1.8).max(1.3);
    let k = Pen {
        p: painter,
        r: rect,
        color,
        w,
    };
    use Icon::*;
    match icon {
        List => {
            k.line(&[(4., 6.), (20., 6.)]);
            k.line(&[(4., 12.), (20., 12.)]);
            k.line(&[(4., 18.), (20., 18.)]);
        }
        Download => {
            k.line(&[(12., 4.), (12., 15.)]);
            k.line(&[(7., 10.), (12., 15.), (17., 10.)]);
            k.line(&[(5., 20.), (19., 20.)]);
        }
        Clock => {
            k.circle(12., 12., 8.);
            k.line(&[(12., 8.), (12., 12.), (15., 14.)]);
        }
        Check => k.line(&[(5., 12.5), (9.5, 17.), (19., 7.)]),
        Alert => {
            k.closed(&[(12., 4.), (21., 20.), (3., 20.)]);
            k.line(&[(12., 10.), (12., 14.)]);
            k.dot(12., 17.2, 0.9);
        }
        Video => {
            k.rect(3., 5., 18., 14., 2.5);
            k.fill(&[(10., 9.5), (14.5, 12.), (10., 14.5)]);
        }
        Music => {
            k.line(&[(9., 17.), (9., 6.), (20., 4.), (20., 15.)]);
            k.circle(7., 17., 2.2);
            k.circle(18., 15., 2.2);
        }
        Image => {
            k.rect(3., 5., 18., 14., 2.);
            k.line(&[(3., 16.), (8., 11.), (12., 15.), (15., 12.), (21., 18.)]);
            k.circle(15.5, 9.5, 1.5);
        }
        Doc => {
            k.closed(&[(7., 3.), (14., 3.), (19., 8.), (19., 21.), (7., 21.)]);
            k.line(&[(14., 3.), (14., 8.), (19., 8.)]);
            k.line(&[(10., 13.), (16., 13.)]);
            k.line(&[(10., 17.), (14., 17.)]);
        }
        Archive => {
            k.rect(6., 3., 12., 18., 1.);
            k.line(&[(12., 3.), (12., 5.)]);
            k.line(&[(12., 7.), (12., 9.)]);
            k.line(&[(12., 11.), (12., 13.)]);
            k.rect(10.5, 14., 3., 4., 0.8);
        }
        Program => {
            k.closed(&[
                (12., 3.),
                (20., 7.5),
                (20., 16.5),
                (12., 21.),
                (4., 16.5),
                (4., 7.5),
            ]);
            k.line(&[(4., 7.5), (12., 12.), (20., 7.5)]);
            k.line(&[(12., 12.), (12., 21.)]);
        }
        Disk => {
            k.circle(12., 12., 8.5);
            k.circle(12., 12., 2.5);
            k.arc(12., 12., 6.0, -90.0, 0.0);
        }
        File => {
            k.closed(&[(7., 3.), (14., 3.), (19., 8.), (19., 21.), (7., 21.)]);
            k.line(&[(14., 3.), (14., 8.), (19., 8.)]);
        }
        Plus => {
            k.line(&[(12., 5.), (12., 19.)]);
            k.line(&[(5., 12.), (19., 12.)]);
        }
        Play => k.fill(&[(8., 5.), (19., 12.), (8., 19.)]),
        Pause => {
            let k2 = Pen { w: w * 1.4, ..k };
            k2.line(&[(9., 6.), (9., 18.)]);
            k2.line(&[(15., 6.), (15., 18.)]);
        }
        Trash => {
            k.line(&[(4., 7.), (20., 7.)]);
            k.line(&[(9., 7.), (9., 4.), (15., 4.), (15., 7.)]);
            k.line(&[(6., 7.), (7., 20.), (17., 20.), (18., 7.)]);
        }
        PauseCircle => {
            k.circle(12., 12., 8.);
            k.line(&[(10., 9.), (10., 15.)]);
            k.line(&[(14., 9.), (14., 15.)]);
        }
        Queue => {
            k.line(&[(4., 7.), (15., 7.)]);
            k.line(&[(4., 12.), (15., 12.)]);
            k.line(&[(4., 17.), (11., 17.)]);
            k.fill(&[(17., 14.), (21., 17.), (17., 20.)]);
        }
        Folder => k.closed(&[
            (3., 7.),
            (3., 18.),
            (21., 18.),
            (21., 8.5),
            (11.5, 8.5),
            (9.5, 6.),
            (3., 6.),
        ]),
        Gauge => {
            k.arc(12., 15., 8., 180.0, 360.0);
            k.line(&[(12., 14.), (15., 10.)]);
        }
        Sun => {
            k.circle(12., 12., 4.);
            for i in 0..8 {
                let a = (i as f32 * 45.0f32).to_radians();
                k.line(&[
                    (12. + 7.5 * a.cos(), 12. + 7.5 * a.sin()),
                    (12. + 9.5 * a.cos(), 12. + 9.5 * a.sin()),
                ]);
            }
        }
        Moon => {
            // crescent = part of the outer circle outside the inner one, then the inner arc back
            let (ox, oy, or) = (12.0f32, 12.0f32, 8.0f32);
            let (ix, iy, ir) = (16.5f32, 8.0f32, 6.5f32);
            let n = 120;
            let outer: Vec<(f32, f32)> = (0..n)
                .map(|i| {
                    let a = std::f32::consts::TAU * i as f32 / n as f32;
                    (ox + or * a.cos(), oy + or * a.sin())
                })
                .collect();
            let outside = |p: &(f32, f32)| ((p.0 - ix).powi(2) + (p.1 - iy).powi(2)).sqrt() > ir;
            if let Some(start) =
                (0..n).find(|&i| outside(&outer[i]) && !outside(&outer[(i + n - 1) % n]))
            {
                let mut pts: Vec<(f32, f32)> = (0..n)
                    .map(|k| outer[(start + k) % n])
                    .take_while(outside)
                    .collect();
                let inner: Vec<(f32, f32)> = (0..n)
                    .map(|i| {
                        let a = std::f32::consts::TAU * i as f32 / n as f32;
                        (ix + ir * a.cos(), iy + ir * a.sin())
                    })
                    .collect();
                let inside_outer =
                    |p: &(f32, f32)| ((p.0 - ox).powi(2) + (p.1 - oy).powi(2)).sqrt() < or;
                if let Some(s2) = (0..n)
                    .find(|&i| inside_outer(&inner[i]) && !inside_outer(&inner[(i + n - 1) % n]))
                {
                    let back: Vec<(f32, f32)> = (0..n)
                        .map(|k| inner[(s2 + k) % n])
                        .take_while(inside_outer)
                        .collect();
                    pts.extend(back.into_iter().rev());
                }
                k.closed(&pts);
            }
        }
        Monitor => {
            k.rect(3., 4., 18., 12., 2.);
            k.line(&[(8., 20.), (16., 20.)]);
            k.line(&[(12., 16.), (12., 20.)]);
        }
        Link => {
            k.line(&[(10., 14.), (14., 10.)]);
            k.line(&[
                (9., 11.5),
                (6., 14.5),
                (6., 17.),
                (7., 18.),
                (9.5, 18.),
                (12.5, 15.),
            ]);
            k.line(&[
                (11.5, 9.),
                (14.5, 6.),
                (17., 6.),
                (18., 7.),
                (18., 9.5),
                (15., 12.5),
            ]);
        }
        Close => {
            k.line(&[(6., 6.), (18., 18.)]);
            k.line(&[(18., 6.), (6., 18.)]);
        }
        Upload => {
            k.line(&[(12., 20.), (12., 9.)]);
            k.line(&[(7., 14.), (12., 9.), (17., 14.)]);
            k.line(&[(5., 4.), (19., 4.)]);
        }
        Plug => {
            k.line(&[(9., 3.), (9., 7.)]);
            k.line(&[(15., 3.), (15., 7.)]);
            k.line(&[
                (7., 7.),
                (17., 7.),
                (17., 11.),
                (15.5, 14.5),
                (12., 16.),
                (8.5, 14.5),
                (7., 11.),
                (7., 7.),
            ]);
            k.line(&[(12., 16.), (12., 21.)]);
        }
    }
}

pub fn for_type(t: crate::model::FileType) -> Icon {
    use crate::model::FileType::*;
    match t {
        Video => Icon::Video,
        Audio => Icon::Music,
        Disk => Icon::Disk,
        Archive => Icon::Archive,
        Document => Icon::Doc,
        Image => Icon::Image,
        Program => Icon::Program,
        Other => Icon::File,
    }
}
