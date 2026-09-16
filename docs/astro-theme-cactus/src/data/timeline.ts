/**
 * Zombie Deck — build timeline data.
 *
 * Each entry is one point in the vertical showcase timeline.
 * Supports text, images, and videos per entry.
 *
 * TODO (replace placeholders with real build content):
 * - Update `date`, `title`, `description` with real milestones
 * - Replace `images[].src` with e.g. `/showcase/<your-photo>.jpg` (put files in `public/showcase/`)
 *   or with repo assets like `../../../assets/images/...`
 * - Replace `video.src` with e.g. `/showcase/<your-clip>.mp4` (put files in `public/showcase/`)
 * - Optional: `links: [{ label, href }]`, `tags: [...]`, `status`
 */

export interface TimelineMediaImage {
	/** Image URL — local (`/showcase/foo.jpg`) or remote */
	src: string;
	/** Alt text for a11y */
	alt: string;
	/** Optional caption shown under the image */
	caption?: string;
}

export interface TimelineMediaVideo {
	/** Video file URL — local (`/showcase/foo.mp4`) or remote */
	src: string;
	/** Poster image shown before playback */
	poster?: string;
	/** Optional caption shown under the video */
	caption?: string;
}

export interface TimelineLink {
	label: string;
	href: string;
}

export type TimelineStatus = "idea" | "prototype" | "building" | "testing" | "done";

export interface TimelineEntry {
	id: string;
	date: string;
	displayDate: string;
	title: string;
	subtitle?: string;
	description: string;
	bullets?: string[];
	images?: TimelineMediaImage[];
	video?: TimelineMediaVideo;
	links?: TimelineLink[];
	tags?: string[];
	status?: TimelineStatus;
}

export const timelineEntries: TimelineEntry[] = [
	{
		id: "idea",
		date: "2026-01-10",
		displayDate: "Day 0 — The Idea",
		title: "What if your street was the game map?",
		subtitle: "Concept & useless-problem framing",
		description:
			"PLACEHOLDER — Zombie Deck started as a joke with a real question: how close is the nearest zombie while jogging past the local chaya kada? We sketched a handheld Pi terminal that turns OpenStreetMap roads + POIs into a live ASCII survival map. No joystick — you are the controller.",
		bullets: [
			"PLACEHOLDER — problem statement: quantify undead pursuit distance",
			"PLACEHOLDER — solution sketch: Pi + TFT + GPS + keyboard",
			"PLACEHOLDER — success criteria: run, or get eaten",
		],
		images: [
			{
				src: "https://picsum.photos/seed/zdeck-idea/800/450",
				alt: "Placeholder sketch of the Zombie Deck concept",
				caption: "PLACEHOLDER — replace with whiteboard / concept sketch photo",
			},
		],
		tags: ["concept", "design"],
		status: "idea",
	},
	{
		id: "prototype-python",
		date: "2026-02-02",
		displayDate: "Build 01 — First Prototype",
		title: "Green dot vs. red dots",
		subtitle: "Python curses prototype in --sim mode",
		description:
			"PLACEHOLDER — first playable prototype in app/cyberdeck.py. Black terminal, green player, red zombies that chase and catch. WASD movement with --sim flag for indoor testing. Tuning knobs: SCALE_M_PER_CELL, ZOMBIE_COUNT, ZOMBIE_SPEED_MPS, CATCH_RADIUS_M.",
		bullets: [
			"PLACEHOLDER — curses rendering loop at TICK_HZ",
			"PLACEHOLDER — zombie chase AI + catch radius",
			"PLACEHOLDER — screenshot of first prototype",
		],
		images: [
			{
				src: "https://picsum.photos/seed/zdeck-proto/800/450",
				alt: "Placeholder screenshot of first prototype",
				caption: "PLACEHOLDER — replace with assets/images/screenshot1.jpeg",
			},
			{
				src: "https://picsum.photos/seed/zdeck-code/800/450",
				alt: "Placeholder code screenshot",
				caption: "PLACEHOLDER — replace with editor screenshot of cyberdeck.py",
			},
		],
		video: {
			src: "https://interactive-examples.mdn.mozilla.net/media/cc0-videos/flower.mp4",
			poster: "https://picsum.photos/seed/zdeck-proto-poster/800/450",
			caption: "PLACEHOLDER — replace with screen-recording of --sim gameplay (.mp4 in public/showcase/)",
		},
		links: [{ label: "app/cyberdeck.py", href: "https://github.com" }],
		tags: ["python", "curses", "sim"],
		status: "prototype",
	},
	{
		id: "osm-maps",
		date: "2026-02-20",
		displayDate: "Build 02 — Real Streets",
		title: "OpenStreetMap integration",
		subtitle: "fetch_map.py + offline map_data.json",
		description:
			"PLACEHOLDER — fetch_map.py downloads roads + POIs around your lat/lon from the Overpass API (stdlib only, no deps). The game then plays offline in the field. Named establishments are drawn so you can run toward shelter.",
		bullets: [
			"PLACEHOLDER — Overpass query: roads + POIs within radius",
			"PLACEHOLDER — offline-first: pre-fetch with internet, play without",
			"PLACEHOLDER — named places + shelter callouts",
		],
		images: [
			{
				src: "https://picsum.photos/seed/zdeck-map/800/450",
				alt: "Placeholder OSM map integration screenshot",
				caption: "PLACEHOLDER — replace with assets/images/screenshot3.jpeg",
			},
		],
		tags: ["osm", "maps", "offline"],
		status: "building",
	},
	{
		id: "gps-move",
		date: "2026-03-05",
		displayDate: "Build 03 — You Are the Joystick",
		title: "Live GPS movement",
		subtitle: "NEO-6M over UART + pynmea2 / pyserial",
		description:
			"PLACEHOLDER — outdoors, your body is the joystick. NEO-6M GPS over UART (9600 baud NMEA) feeds player position. Indoors, fall back to --sim WASD. TFT occupies hardware UART so GPIO16 bit-bang mode via pigpio is supported.",
		bullets: [
			"PLACEHOLDER — GPS parsing: pynmea2 + pyserial",
			"PLACEHOLDER — wiring: TX → GPIO16, GND, VCC",
			"PLACEHOLDER — field test video: walking = moving on map",
		],
		images: [
			{
				src: "https://picsum.photos/seed/zdeck-gps/800/450",
				alt: "Placeholder GPS module photo",
				caption: "PLACEHOLDER — replace with NEO-6M wiring photo",
			},
		],
		video: {
			src: "https://interactive-examples.mdn.mozilla.net/media/cc0-videos/flower.mp4",
			poster: "https://picsum.photos/seed/zdeck-gps-poster/800/450",
			caption: "PLACEHOLDER — replace with field-test GPS walk video",
		},
		tags: ["gps", "hardware", "neo-6m"],
		status: "testing",
	},
	{
		id: "rust-port",
		date: "2026-03-25",
		displayDate: "Build 04 — Rust Port",
		title: "Faster TUI, same hunger",
		subtitle: "ratatui + crossterm binaries",
		description:
			"PLACEHOLDER — Rust port for speed: zdeck-game / zdeck-gps / zdeck-fetch / zdeck-run / zdeck-cardkb. Same game logic, snappier terminal UI. Cross-compile for Pi 3 (arm64 + armv7) via Docker in rust/build-pi.sh.",
		bullets: [
			"PLACEHOLDER — ratatui + crossterm TUI",
			"PLACEHOLDER — clap CLI, serde_json maps",
			"PLACEHOLDER — side-by-side Python vs Rust demo",
		],
		images: [
			{
				src: "https://picsum.photos/seed/zdeck-rust/800/450",
				alt: "Placeholder Rust TUI screenshot",
				caption: "PLACEHOLDER — replace with zdeck-run terminal screenshot",
			},
		],
		tags: ["rust", "ratatui", "tui"],
		status: "building",
	},
	{
		id: "deck-assembly",
		date: "2026-04-12",
		displayDate: "Build 05 — The Cyber Deck",
		title: "Pi + TFT + CardKB assembly",
		subtitle: "Schematic, circuit, build photos",
		description:
			"PLACEHOLDER — handheld assembly: Pi 3, 3.5in TFT over SPI, M5Stack CardKB over I2C (0x5F) exposed via uinput, USB power bank. Boot-to-game via tty1 login hook (zdeck-main.sh). CardKB setup + diagnostics in cardkb/.",
		bullets: [
			"PLACEHOLDER — schematic + circuit photos",
			"PLACEHOLDER — CardKB I2C wiring: SDA→GPIO2, SCL→GPIO3",
			"PLACEHOLDER — build progress photos op1 → op4 → final",
		],
		images: [
			{
				src: "https://picsum.photos/seed/zdeck-circuit/800/450",
				alt: "Placeholder circuit photo",
				caption: "PLACEHOLDER — replace with assets/images/circuit.jpeg",
			},
			{
				src: "https://picsum.photos/seed/zdeck-build/800/450",
				alt: "Placeholder build photo",
				caption: "PLACEHOLDER — replace with assets/images/op1.jpeg",
			},
			{
				src: "https://picsum.photos/seed/zdeck-final/800/450",
				alt: "Placeholder final deck photo",
				caption: "PLACEHOLDER — replace with assets/images/zdeck.jpeg",
			},
		],
		video: {
			src: "https://interactive-examples.mdn.mozilla.net/media/cc0-videos/flower.mp4",
			poster: "https://picsum.photos/seed/zdeck-assembly-poster/800/450",
			caption: "PLACEHOLDER — replace with assembly timelapse video",
		},
		tags: ["hardware", "pi3", "tft", "cardkb"],
		status: "building",
	},
	{
		id: "demo-day",
		date: "2026-05-01",
		displayDate: "Now — Demo Day",
		title: "Run for your life",
		subtitle: "Live demo + what's next",
		description:
			"PLACEHOLDER — final demo: power on, load locality, spawn zombies, run. Add your Drive demo link, final build photo, and reflections here. Next: sound, haptics, multiplayer horde mode, bigger TFT?",
		images: [
			{
				src: "https://picsum.photos/seed/zdeck-demo/800/450",
				alt: "Placeholder demo photo",
				caption: "PLACEHOLDER — replace with final demo photo",
			},
		],
		video: {
			src: "https://interactive-examples.mdn.mozilla.net/media/cc0-videos/flower.mp4",
			poster: "https://picsum.photos/seed/zdeck-demo-poster/800/450",
			caption: "PLACEHOLDER — replace with Drive demo video (download to public/showcase/demo.mp4)",
		},
		links: [
			{ label: "Demo video (Drive)", href: "https://drive.google.com/" },
			{ label: "Source", href: "https://github.com" },
		],
		tags: ["demo", "final"],
		status: "done",
	},
];
