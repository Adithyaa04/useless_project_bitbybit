<img width="1280" height="640" alt="git (1)" src="https://github.com/user-attachments/assets/8920b256-2ba8-4988-b824-5351134eb4bd" />



# [Project Name] 🎯


## Basic Details
### Team Name: [Bit By Bit]


### Team Members
- Member 1: Abhijith R - NSS College Of Engineering Palakkad
- Member 2: Adithya Vijay - NSS College Of Engineering Palakkad


### Project Description
A hardware-based zombie chase game that runs on a handheld Raspberry Pi terminal. Run for your life as zombies chase you!!

### The Problem (that doesn't exist)
Zombie apocalypses are real. Ofcourse EVERYBODY need to outrun the undead through their own neighborhood on a Tuesday evening. Humanity remains criminally under-equipped to know exactly how close a horde of zombies currently is, in meters, while jogging past the local chaya kada.

### The Solution (that nobody asked for)
We made a hand-held Cyber Deck using Raspberry Pi, and since your street is a live zombie-infested map, and you are the controller. No joystick, no WASD, you outrun the zombies by, well, outrunning them.

## Technical Details
### Technologies/Components Used
For Software:
- curses (terminal rendering engine for the TFT display)
- argparse, json, math, random, threading (standard library)
- pynmea2 (parses NMEA GPS sentences)
- pyserial (reads the GPS module over UART)
- OpenStreetMap Overpass API (source of real road + point-of-interest data)

For Hardware:
- [List main components]
- [List specifications]
- [List tools required]

### Implementation
For Software:
# Installation
- pip install pynmea2 pyserial --break-system-packages

# Run
- python3 fetch_map1.py --lat <your-lat> --lon <your-lon> --radius 300
- python3 zombie_cyberdeck.py --sim

# Run (for real, outdoors, with GPS wired up)

### Project Documentation
For Software:

# Screenshots (Add at least 3)
![Screenshot1](./screenshot1.jpg)
First prototype for the application. just a human (green dot) and zombies(red) in a black background, and they can interact with eachother

![Screenshot2](./screenshot2.jpg)
LOC, integrating OpenStreetMap, pynmea2 pyserial and other modules necessary for the functioning

![Screenshot3](./screenshot3.jpg)
Better output model, which can show the names of establishments and help the human to shelter

For Hardware:

# Schematic & Circuit
![Circuit](Add your circuit diagram here)
*Add caption explaining connections*

![Schematic](Add your schematic diagram here)
*Add caption explaining the schematic*

# Build Photos
![Components](Add photo of your components here)
*List out all components shown*

![Build](Add photos of build process here)
*Explain the build steps*

![Final](Add photo of final product here)
*Explain the final build*

### Project Demo
# Video
[Add your demo video link here]
*Explain what the video demonstrates*

# Additional Demos
[Add any extra demo materials/links]

## Team Contributions
- [Name 1]: [Specific contributions]
- [Name 2]: [Specific contributions]
- [Name 3]: [Specific contributions]

---
Made with ❤️ at TinkerHub Useless Projects 

![Static Badge](https://img.shields.io/badge/TinkerHub-24?color=%23000000&link=https%3A%2F%2Fwww.tinkerhub.org%2F)
![Static Badge](https://img.shields.io/badge/UselessProjects--26-26?link=https%3A%2F%2Ftinkerhub.org%2Fevents%2F1M8ORET9A1%2Fuseless-projects-3.0)



