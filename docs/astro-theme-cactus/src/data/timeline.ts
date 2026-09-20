/**
 * Zombie Deck — build timeline data.
 *
 * Each entry is one point in the vertical showcase timeline.
 * Supports text, images, and videos per entry.
 *
 * Media lives in `public/media1/` (served as `/media1/...`).
 * To swap an image/video, drop the file in `public/media1/`
 * and update `images[].src` / `video.src` below.
 */

export interface TimelineMediaImage {
	/** Image URL — local (`/media1/foo.JPG`) or remote */
	src: string;
	/** Alt text for a11y */
	alt: string;
	/** Optional caption shown under the image */
	caption?: string;
}

export interface TimelineMediaVideo {
	/** Video file URL — local (`/media1/foo.MOV`) or remote */
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
			"Zombie Deck started as a joke with a real question: how close is the nearest zombie while jogging past the local chaya kada? We sketched a handheld Pi terminal that turns OpenStreetMap roads + POIs into a live ASCII survival map. No joystick — you are the controller.",
		bullets: [
			"Problem statement: quantify undead pursuit distance",
			"Solution sketch: Pi + TFT + GPS + keyboard",
			"Success criteria: run, or get eaten",
		],
		images: [
			{
				src: "/media1/cutepic.jpg",
				alt: "Team cute pic",
				caption: "The team that decided jogging needed zombies",
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
			"First playable prototype in app/cyberdeck.py. Black terminal, green player, red zombies that chase and catch. WASD movement with --sim flag for indoor testing. Tuning knobs: SCALE_M_PER_CELL, ZOMBIE_COUNT, ZOMBIE_SPEED_MPS, CATCH_RADIUS_M.",
		bullets: [
			"Curses rendering loop at TICK_HZ",
			"Zombie chase AI + catch radius",
			"Screenshot of first prototype",
		],
		images: [
			{
				src: "/media1/first.jpeg",
				alt: "First build session",
				caption: "First build session",
			},
			{
				src: "/media1/aurafarm.JPG",
				alt: "Talking through the prototype",
				caption: "Talking through the prototype",
			},
		],
		video: {
			src: "/media1/interviewreal.mp4",
			poster: "/media1/building.JPG",
			caption: "Team interview on the build",
		},
		links: [
			{
				label: "app/cyberdeck.py",
				href: "https://github.com/Adithyaa04/useless_project_bitbybit/blob/journal/app/cyberdeck.py",
			},
		],
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
			"fetch_map.py downloads roads + POIs around your lat/lon from the Overpass API (stdlib only, no deps). The game then plays offline in the field. Named establishments are drawn so you can run toward shelter.",
		bullets: [
			"Overpass query: roads + POIs within radius",
			"Offline-first: pre-fetch with internet, play without",
			"Named places + shelter callouts",
		],
		video: {
			src: "/media1/osm.mp4",
			poster: "/media1/aurafarm.JPG",
			caption: "Mapping the neighbourhood — OSM fetch in action",
		},
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
			"Outdoors, your body is the joystick. NEO-6M GPS over UART (9600 baud NMEA) feeds player position. Indoors, fall back to --sim WASD. TFT occupies hardware UART so GPIO16 bit-bang mode via pigpio is supported.",
		bullets: [
			"GPS parsing: pynmea2 + pyserial",
			"Wiring: TX → GPIO16, GND, VCC",
			"Field test: walking = moving on map",
		],
		images: [
			{
				src: "/media1/DSC04539%20(1).JPG",
				alt: "Field testing the Zombie Deck GPS",
				caption: "Out in the field with the deck",
			},
		],
		video: {
			src: "/media1/gpswork.mp4",
			poster: "/media1/DSC04539%20(1).JPG",
			caption: "Talking through the GPS field tests",
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
			"Rust port for speed: zdeck-game / zdeck-gps / zdeck-fetch / zdeck-run / zdeck-cardkb. Same game logic, snappier terminal UI. Cross-compile for Pi 3 (arm64 + armv7) via Docker in rust/build-pi.sh.",
		bullets: [
			"ratatui + crossterm TUI",
			"clap CLI, serde_json maps",
			"Side-by-side Python vs Rust demo",
		],
		images: [
			{
				src: "/media1/presenting2.JPG",
				alt: "Presenting the Rust port",
				caption: "Presenting the faster TUI",
			},
		],
		video: {
			src: "/media1/rustintegrated.mp4",
			poster: "/media1/presenting2.JPG",
			caption: "Rust TUI integrated and running",
		},
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
			"Handheld assembly: Pi 3, 3.5in TFT over SPI, M5Stack CardKB over I2C (0x5F) exposed via uinput, USB power bank. Boot-to-game via tty1 login hook (zdeck-main.sh). CardKB setup + diagnostics in cardkb/.",
		bullets: [
			"Schematic + circuit photos",
			"CardKB I2C wiring: SDA→GPIO2, SCL→GPIO3",
			"Build progress photos",
		],
		images: [
			{
				src: "/media1/deck.jpeg",
				alt: "The assembled deck",
				caption: "The assembled deck",
			},
			{
				src: "/media1/assembly.jpeg",
				alt: "Putting the hardware together",
				caption: "Putting the hardware together",
			},
			{
				src: "/media1/presenting.JPG",
				alt: "Builders with their creation",
				caption: "Builders with their creation",
			},
		],
		video: {
			src: "/media1/interviewreal.mp4",
			poster: "/media1/presenting3.JPG",
			caption: "Interview on the deck assembly",
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
			"Final demo: power on, load locality, spawn zombies, run. Next: sound, haptics, multiplayer horde mode, bigger TFT?",
		images: [
			{
				src: "/media1/presenting.JPG",
				alt: "Zombie Deck demo day",
				caption: "Demo day",
			},
		],
		video: {
			src: "/media1/demoday.mp4",
			poster: "/media1/presenting2.JPG",
			caption: "Demo-day video",
		},
		links: [
			{ label: "Demo video (Drive)", href: "https://drive.google.com/" },
			{ label: "Source", href: "https://github.com/Adithyaa04/useless_project_bitbybit" },
		],
		tags: ["demo", "final"],
		status: "done",
	},
	{
		id: "top-5",
		date: "2026-05-10",
		displayDate: "Got Selected in Top 5!!",
		title: "Got Selected in Top 5!!",
		subtitle: "Useless Projects 3.0 finals",
		description:
			"Zombie Deck made it to the Top 5 at TinkerHub Useless Projects 3.0! All that running from imaginary zombies paid off. Thanks to everyone who cheered, played the demo, and believed that jogging needed zombies.",
		bullets: [
			"Selected among the Top 5 projects",
			"Live demo + presenting to the judges",
			"Team celebration",
		],
		images: [
			{
				src: "/media1/top5.jpg",
				alt: "Top 5 selection",
				caption: "Top 5!!",
			},
			{
				src: "/media1/group%20(2).JPG",
				alt: "Team group photo",
				caption: "The team",
			},
		],
		tags: ["top5", "finals", "celebration"],
		status: "done",
	},
];
