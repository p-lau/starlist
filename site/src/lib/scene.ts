// The hero scene's geometry: a star seen close up, and satellites that really
// orbit it. Everything here is plain 3D maths that ends in SVG path strings, so
// the same code draws the first frame on the server and every frame after.
//
// World: the star's centre is the origin, x right, y up, z toward the viewer.

type V = [number, number, number];

/** Size of the drawing (the SVG viewBox is W x H). It is cropped to fit the screen. */
export const W = 1600;
export const H = 1000;
/** The star's centre, far below the drawing, so only a slice of its edge shows:
 *  rising from the bottom left, highest right of centre. */
export const CX = 1050;
export const CY = 2160;
/** The star's radius. */
export const R = 1500;
/** Distance from the viewer to the star's centre. Smaller means stronger perspective. */
const CAMERA = 6000;

const add = (a: V, b: V): V => [a[0] + b[0], a[1] + b[1], a[2] + b[2]];
const mul = (a: V, k: number): V => [a[0] * k, a[1] * k, a[2] * k];
const dot = (a: V, b: V) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
const rotX = ([x, y, z]: V, a: number): V => {
	const c = Math.cos(a);
	const s = Math.sin(a);
	return [x, y * c - z * s, y * s + z * c];
};
const rotZ = ([x, y, z]: V, a: number): V => {
	const c = Math.cos(a);
	const s = Math.sin(a);
	return [x * c - y * s, x * s + y * c, z];
};

const n = (v: number) => v.toFixed(1);

/** Perspective: things nearer the viewer are drawn larger and further from the centre. */
function project(v: V): [number, number, number] {
	const s = CAMERA / (CAMERA - v[2]);
	return [CX + v[0] * s, CY - v[1] * s, s];
}
const at = (v: V) => {
	const p = project(v);
	return `${n(p[0])} ${n(p[1])}`;
};
const poly = (pts: V[], close = true) => 'M' + pts.map(at).join('L') + (close ? 'Z' : '');
const seg = (a: V, b: V) => `M${at(a)}L${at(b)}`;

// ---------------------------------------------------------------- the star

// The star's axis leans toward the viewer, so its pole is in view just right of centre.
const tiltStar = (v: V) => rotZ(rotX(v, 0.6), -0.116);

/**
 * The star's grid of meridians and parallels, turned by `spin` radians about
 * its axis. Only the half facing the viewer is drawn.
 */
export function starGrid(spin: number): string {
	let d = '';
	const curve = (point: (i: number) => V, steps: number) => {
		let pen = false;
		for (let i = 0; i <= steps; i++) {
			const p = tiltStar(point(i));
			if (p[2] <= 0) {
				pen = false;
				continue;
			}
			d += `${pen ? 'L' : 'M'}${n(CX + p[0] * R)} ${n(CY - p[1] * R)}`;
			pen = true;
		}
	};
	const MERIDIANS = 24;
	for (let m = 0; m < MERIDIANS; m++) {
		const lon = spin + (m / MERIDIANS) * Math.PI * 2;
		curve((i) => {
			// only the northern half reaches the visible slice
			const lat = (i / 30) * (Math.PI / 2);
			return [Math.cos(lat) * Math.cos(lon), Math.sin(lat), Math.cos(lat) * Math.sin(lon)];
		}, 30);
	}
	for (const deg of [10, 25, 40, 55, 68, 79]) {
		const lat = (deg * Math.PI) / 180;
		curve((i) => {
			const lon = (i / 160) * Math.PI * 2;
			return [Math.cos(lat) * Math.cos(lon), Math.sin(lat), Math.cos(lat) * Math.sin(lon)];
		}, 160);
	}
	return d;
}

/** Short rays standing off the star's edge, with lengths that look irregular.
 *  `set` 0 or 1 picks every other ray, so the two halves can flicker in turn. */
export function coronaRays(set: 0 | 1): string {
	let d = '';
	// Only the top of the circle is ever in view (60 to 140 degrees from the right).
	for (let i = set; i <= 260; i += 2) {
		const a = ((60 + (i / 260) * 80) * Math.PI) / 180;
		const len = 6 + 40 * Math.abs(Math.sin(i * 12.9898) * Math.cos(i * 4.1414)) ** 1.6;
		const r0 = R + 18;
		const x = Math.cos(a);
		const y = Math.sin(a);
		d += `M${n(CX + x * r0)} ${n(CY - y * r0)}L${n(CX + x * (r0 + len))} ${n(CY - y * (r0 + len))}`;
	}
	return d;
}

/** Loops of plasma arching off the edge at a few places (angles in degrees from the right). */
export function prominences(): string {
	let d = '';
	for (const [deg, width, height] of [
		[79, 1.5, 46],
		[86, 1, 30],
		[103, 1.8, 56],
		[124, 1.3, 40]
	]) {
		const a0 = ((deg - width) * Math.PI) / 180;
		const a1 = ((deg + width) * Math.PI) / 180;
		const am = (deg * Math.PI) / 180;
		const p = (a: number, r: number) => `${n(CX + Math.cos(a) * r)} ${n(CY - Math.sin(a) * r)}`;
		d += `M${p(a0, R)}Q${p(am, R + height * 2)} ${p(a1, R)}`;
		d += `M${p(a0 + 0.004, R)}Q${p(am, R + height * 1.3)} ${p(a1 - 0.004, R)}`;
	}
	return d;
}

// ---------------------------------------------------------- the satellites

export type Orbit = {
	/** Orbit radius. */
	radius: number;
	/** How far the orbit is tipped toward the viewer, then rolled, in radians. */
	tip: number;
	roll: number;
	/** Seconds for one lap. */
	period: number;
	/** Size of its satellites. */
	scale: number;
	/** Where on the lap each of its satellites starts (0 to 1). */
	phases: number[];
};

// Two orbits that pass high over the star's right side. A satellite slides
// in from the right, arcs over the top roughly following the curve of the
// horizon, and sets behind the star's edge. It is never drawn over the
// star's face, and never crosses the text on the left. (The angles were
// found by searching for paths that do all that while staying as parallel
// to the horizon as possible.)
export const orbits: Orbit[] = [
	{ radius: 1750, tip: -2.8, roll: 1.5, period: 60, scale: 1.15, phases: [0, 0.34, 0.67] },
	{ radius: 1680, tip: -2.65, roll: 1.42, period: 84, scale: 0.72, phases: [0.17, 0.67] }
];

/** Every satellite: which orbit it is on and where it starts. */
export const fleet = orbits.flatMap((orbit, index) => orbit.phases.map((phase) => ({ orbit, index, phase })));

const frameOf = (o: Orbit) => (v: V) => rotZ(rotX(v, o.tip), o.roll);

/** The orbit's line, split into the part behind the star and the part in front. */
export function orbitTrack(o: Orbit): { back: string; front: string } {
	const turn = frameOf(o);
	const out = { back: '', front: '' };
	let last: 'back' | 'front' | undefined;
	const STEPS = 360;
	for (let i = 0; i <= STEPS; i++) {
		const a = (i / STEPS) * Math.PI * 2;
		const p = turn([Math.cos(a) * o.radius, 0, Math.sin(a) * o.radius]);
		const side = p[2] > 0 ? 'front' : 'back';
		out[side] += (last === side ? 'L' : 'M') + at(p);
		last = side;
	}
	return out;
}

// The satellite, in its own coordinates: x along the solar panels, y the way
// it is flying, z toward the star. Half-sizes of the body:
const BX = 13;
const BY = 13;
const BZ = 17;

/** One drawn part. `light` (0 to 1) is how much starlight falls on it as seen from here. */
export type Part = { kind: 'panel' | 'face' | 'dish' | 'rig'; d: string; light: number };
export type Satellite = {
	/** In front of the star (true) or behind it. */
	front: boolean;
	/** How large it appears, for line widths. */
	size: number;
	/** Its parts, furthest first, so nearer parts cover them. */
	parts: Part[];
	/** Where its beacon is, and how large. */
	beacon: [number, number, number];
};

const EMPTY = { size: 1, parts: [], beacon: [0, 0, 0] as [number, number, number] };

/** Where a satellite is at `time` seconds and what it looks like from here. */
export function where(o: Orbit, phase: number, time: number) {
	const a = (time / o.period + phase) * Math.PI * 2;
	const pos = mul(frameOf(o)([Math.cos(a), 0, Math.sin(a)]), o.radius);
	return [...project(pos), pos[2]];
}

export function satellite(o: Orbit, phase: number, time: number): Satellite {
	const turn = frameOf(o);
	const a = (time / o.period + phase) * Math.PI * 2;
	const out: V = turn([Math.cos(a), 0, Math.sin(a)]);
	const pos = mul(out, o.radius);

	// Nothing to draw while it is out of the picture or behind the star.
	const [sx, sy, ss] = project(pos);
	const margin = 130 * o.scale * ss;
	const hidden = pos[2] <= 0 && Math.hypot(sx - CX, sy - CY) < R - margin;
	if (hidden || sx < -margin || sx > W + margin || sy < -margin || sy > H + margin) {
		return { front: pos[2] > 0, ...EMPTY };
	}

	// It keeps one face to the star, as real ones do: so as it goes round we
	// see its front, then its side, then its back.
	const X = turn([0, 1, 0]);
	const Y = turn([-Math.sin(a), 0, Math.cos(a)]);
	const Z = mul(out, -1);
	const k = o.scale;
	const w = (x: number, y: number, z: number): V =>
		add(pos, add(add(mul(X, x * k), mul(Y, y * k)), mul(Z, z * k)));
	const eye: V = [0, 0, CAMERA];
	const depth = (p: V) => p[2];

	const items: { z: number; part: Part }[] = [];

	// Solar panels: a frame, the lines between cells, and the boom that holds it.
	for (const side of [-1, 1]) {
		const x0 = side * 24;
		const x1 = side * 88;
		let d = poly([w(x0, -17, 0), w(x1, -17, 0), w(x1, 17, 0), w(x0, 17, 0)]);
		for (let c = 1; c < 6; c++) {
			const x = x0 + ((x1 - x0) * c) / 6;
			d += seg(w(x, -17, 0), w(x, 17, 0));
		}
		d += seg(w(x0, 0, 0), w(x1, 0, 0));
		// The cells face the star. When we see that side, the starlight glints off them.
		const centre = w((x0 + x1) / 2, 0, 0);
		const toEye = add(eye, mul(centre, -1));
		const glint = Math.max(0, dot(Z, toEye) / Math.hypot(...toEye));
		items.push({ z: depth(centre), part: { kind: 'panel', d, light: glint } });
	}

	// Booms and antenna.
	let rig = seg(w(-BX, 0, 0), w(-24, 0, 0)) + seg(w(BX, 0, 0), w(24, 0, 0));
	rig += seg(w(-BX, -6, 0), w(-24, 0, 0)) + seg(w(BX, -6, 0), w(24, 0, 0));
	rig += seg(w(-BX, 6, 0), w(-24, 0, 0)) + seg(w(BX, 6, 0), w(24, 0, 0));
	const tip = w(7, BY + 26, -8);
	rig += seg(w(7, BY, -8), tip);
	rig += poly([w(7, BY + 26, -11), w(10, BY + 29, -8), w(7, BY + 32, -8), w(4, BY + 29, -8)]);
	items.push({ z: depth(pos) - 0.5, part: { kind: 'rig', d: rig, light: 0 } });

	// Body: a box. Only the faces turned toward the viewer are drawn, which is
	// what makes it read as solid. Faces turned toward the star below are lit
	// warm by it; faces turned away stay dark.
	const faces: [V, V, V][] = [
		[[1, 0, 0], [0, 1, 0], [0, 0, 1]],
		[[-1, 0, 0], [0, 1, 0], [0, 0, 1]],
		[[0, 1, 0], [1, 0, 0], [0, 0, 1]],
		[[0, -1, 0], [1, 0, 0], [0, 0, 1]],
		[[0, 0, 1], [1, 0, 0], [0, 1, 0]],
		[[0, 0, -1], [1, 0, 0], [0, 1, 0]]
	];
	const half: V = [BX, BY, BZ];
	for (const [normal, e1, e2] of faces) {
		const corner = (s1: number, s2: number, inset = 1): V => {
			const l: V = [0, 0, 0];
			for (let i = 0; i < 3; i++) l[i] = (normal[i] + e1[i] * s1 * inset + e2[i] * s2 * inset) * half[i];
			return w(l[0], l[1], l[2]);
		};
		const centre = w(normal[0] * BX, normal[1] * BY, normal[2] * BZ);
		const facing = add(add(mul(X, normal[0]), mul(Y, normal[1])), mul(Z, normal[2]));
		if (dot(facing, add(centre, mul(eye, -1))) >= 0) continue;
		const d =
			poly([corner(-1, -1), corner(1, -1), corner(1, 1), corner(-1, 1)]) +
			poly([corner(-1, -1, 0.62), corner(1, -1, 0.62), corner(1, 1, 0.62), corner(-1, 1, 0.62)]);
		const light = Math.max(0, dot(facing, Z)) * 0.85 + 0.15 * (1 - Math.abs(dot(facing, Y)));
		items.push({ z: depth(centre), part: { kind: 'face', d, light } });
	}

	// Dish, on the side away from the star: a rim, a throat and the feed.
	const ring = (radius: number, z: number) => {
		const pts: V[] = [];
		for (let i = 0; i < 20; i++) {
			const t = (i / 20) * Math.PI * 2;
			pts.push(w(Math.cos(t) * radius, Math.sin(t) * radius, z));
		}
		return pts;
	};
	const rim = ring(17, -BZ - 11);
	const throat = ring(5, -BZ);
	let dish = poly(rim) + poly(throat);
	for (let i = 0; i < 20; i += 5) dish += seg(throat[i], rim[i]);
	dish += seg(w(0, 0, -BZ), w(0, 0, -BZ - 21));
	for (let i = 2; i < 20; i += 7) dish += seg(rim[i], w(0, 0, -BZ - 21));
	items.push({ z: depth(w(0, 0, -BZ - 12)), part: { kind: 'dish', d: dish, light: 0 } });

	items.sort((p, q) => p.z - q.z);

	// The beacon sits on the antenna's tip, so nothing ever covers it.
	const lamp = project(w(7, BY + 33, -8));
	return {
		front: pos[2] > 0,
		size: project(pos)[2] * k,
		parts: items.map((i) => i.part),
		beacon: [lamp[0], lamp[1], 2.6 * k * lamp[2]]
	};
}
