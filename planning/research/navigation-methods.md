Agent completed. agent_id: navigation-research, agent_type: research, status: completed, description: Research 1600-1720 navigation, elapsed: 695s, total_turns: 0, duration: 688s

I now have enough source material across all key topics. Let me synthesize the complete research report.

---

# Historical Navigation Research: Caribbean & Atlantic, 1600–1720
## For Pirate-Sim Navigation Model

---

## SUMMARY

Navigation in the 1600–1720 period was a precise science in one axis (latitude) and a near-total guess in the other (longitude). Navigators had accurate celestial instruments to determine latitude to within ½–1° (~30–60 NM), but no reliable way to determine longitude, leading to east-west errors routinely in the range of 1–5° (60–300 NM) after a transatlantic crossing. The entire colonial-era Atlantic routing system was designed around this asymmetry: sail to a known latitude, then run along it until you bump into land. Caribbean inter-island navigation was largely visual pilotage — the chart problem barely mattered when islands were only 50–200 NM apart and visible from 15–25 NM. For the simulation, the **core model** should be: ships know their latitude reasonably well (±30–60 NM), have degrading dead-reckoning longitude (±60–300 NM accumulating error), use landmark fixes to reset both, and follow prescribed routes that exploit wind patterns rather than pathfinding optimally.

---

## 1. NAVIGATION INSTRUMENTS & METHODS

### 1.1 Celestial Instruments (for Latitude)

**Cross-staff (Jacob's Staff)**
- **Period**: In active use 1500–~1650, but fading by 1620s as backstaff superseded it
- **Operation**: The navigator faced the sun (or Polaris) and slid a transom along the staff until one end touched the horizon and the other the celestial body. The angle was read from graduation marks.
- **Critical flaw**: Required looking directly at the sun, which was painful and inaccurate. Eye damage was a real risk. Accuracy degraded at sun altitudes above ~45°. In practice: **±1–2° accuracy** (60–120 NM latitude error).
- **Polaris use**: At night, measuring Polaris altitude gives latitude directly (no sun-staring problem). The cross-staff could achieve ±0.5° on Polaris in calm conditions — about **±30 NM latitude accuracy**.
- **Source**: Wikipedia, "Cross-staff": "The cross-staff, when used for astronomical observations, was also referred to as a *radius astronomicus*... used to determine the angle between the horizon and Polaris or the sun to determine a vessel's latitude."

**Backstaff (Davis Quadrant)**
- **Period**: Invented 1594 by John Davis; dominant instrument in our era (1600–1731 when sextant arrived). The standard English navigation instrument by 1620.
- **Operation**: Navigator stood *with back to the sun*, measuring the shadow cast by an upper vane onto a horizon vane, while sighting the horizon with a lower sight vane. Eliminated the sun-staring problem completely.
- **Accuracy**: The later "Davis quadrant" form (mid-17th century) used two arcs — small upper arc to set shadow vane, large lower arc for precision reading with transversals. Achievable accuracy: **±0.25–0.5°** (±15–30 NM latitude).
- **Practical reality**: Achieving that accuracy required calm seas (a moving deck degraded readings), clear horizon, and skill. At sea, experienced navigators expected **±0.5°–1°** (±30–60 NM).
- **Flamsteed glass variation**: A lens replaced the shadow vane for hazy/overcast conditions, maintaining accuracy on cloudy days when a shadow couldn't be cast.
- **Source**: Wikipedia, "Backstaff": "This form evolved by the mid-17th century... The large arc, in later years, was marked with transversals to allow the arc to be read to greater accuracy."

**Mariner's Astrolabe**
- **Period**: Primary instrument ~1480–1600; largely obsolete by 1620 for skilled navigators but still carried as backup.
- **Operation**: A heavy brass ring hung vertically; navigator sighted the sun or star through two pinholes and read altitude on the graduated ring.
- **Accuracy**: Heavily dependent on sea state; the weight (2–5 kg) helped it hang plumb, but wave motion caused errors of **±2–3°** routinely. Useful mainly in very calm conditions.
- **In Caribbean waters**: The brisk trade winds and Atlantic swells meant the astrolabe was a poor performer underway. Best used anchored or on land.

**Noon Sight (Meridian Altitude)**
- The primary daily latitude fix for all period navigators, regardless of instrument.
- **Method**: Watch the sun climb (or star culminate). At noon local time, the sun reaches its maximum altitude (meridian transit). **Latitude = 90° − (noon altitude) + solar declination**. Declination tables (*ruttiers*, *almanacs*) provided the correction for time of year.
- **Solar declination tables**: Accuracy was ±1–2 arcminutes in 17th-century almanacs (e.g., Flamsteed's corrections). Close enough.
- **Key insight for simulation**: A noon sight gives a **latitude line** — a horizontal line on the chart. The navigator knows exactly which parallel they're on, but their longitude along that line is still dead reckoning.

---

### 1.2 Magnetic Compass

**Standard equipment on every ship.** But with systematic errors:

**Magnetic Declination (Variation)**
- The angle between magnetic north and true north varies by location and changes over time.
- In the Caribbean (c.1700): Roughly **5–8° W** variation. I.e., a compass pointing "north" was actually pointing ~6° west of true north.
- This was *known* by skilled navigators who used variation tables or corrected by Polaris observation. Unskilled navigators or those with outdated tables could have uncorrected errors of **3–6°**, translating to systematic course errors.
- Edmund Halley published the first magnetic declination chart for the Atlantic in **1700** — the immediate predecessors of our simulation period were navigating without this chart.
- Caribbean-specific c.1650: Declination ranged from ~3°W (eastern Caribbean, near Barbados) to ~8°W (Gulf of Mexico, near Veracruz). A navigator using a chart calibrated in Spain (~0° declination in 1600) and sailing to Veracruz would have an uncorrected 8° error unless they knew to compensate.
- **Deviation** (local compass error from the ship's iron): Less of an issue for wooden ships than modern steel vessels, but iron cannons near the compass binnacle could cause **1–3° local deviation**. Best practice was to mount the binnacle on the centerline away from guns.
- **Compass accuracy (net, in practice)**: **±2–5°** for heading, which compounds over distance. Over a 1,000 NM voyage with a 3° uncorrected error, you'd arrive ~52 NM off your intended latitude line even if you held that heading perfectly.
- **Source**: Wikipedia, "Magnetic Declination": "Reports of measured magnetic declination for distant locations became commonplace in the 17th century, and Edmund Halley made a map of declination for the Atlantic Ocean in 1700."

---

### 1.3 Chip Log (Speed Measurement)

**Construction**: A quarter-circle of wood (5–6" radius), weighted on the curved edge with lead to float upright. Attached to a log-line knotted at regular intervals, wound on a reel.

**Operation**:
1. Throw the chip log overboard off the stern
2. The chip acts as a drogue — stays roughly stationary as ship moves away
3. Count how many knots (tied knots in the line) pass through your hands in a fixed time period (28 or 30 seconds with a sandglass)
4. That count = speed in knots

**The knot spacing**: Originally calibrated to 7 fathoms (42 feet) per 30-second glass; later standardized to ~47.25 feet (14.4m) per 28-second glass to match the nautical mile (6,076 feet). The two standards coexisted throughout our period, causing calibration confusion.

**Accuracy problems**:
1. **Sandglass error**: A 28-second sandglass could be off by 1–3 seconds (3–10% error). Not standardized between ships.
2. **Line tension/drag**: In heavy weather, the line sagged and dragged, giving falsely high readings.
3. **Current**: The chip log measures speed *through the water*, not *over the ground*. A 2-knot Gulf Stream current hitting you sideways was invisible to the log.
4. **Wave action**: Reading the knots on a heaving deck introduced human error.
5. **Net accuracy**: **±10–20% on speed readings**, translating to proportional errors in dead-reckoning distance.
- **Source**: Wikipedia, "Chip log": "Many ships used knots spaced 8 fathoms (48 feet) apart, while other ships used the 7-fathom prescription... the accuracy of dead reckoning can be increased significantly by using other, more reliable methods to get a new fix."

**Leeway not measured**: The chip log gives speed through water on the heading axis, but ships making leeway (drifting sideways due to wind) would travel a different actual course. Leeway (typically **3–8°** for square-riggers in a beam wind, up to 15° close-hauled) was estimated visually by experienced navigators from the angle of the wake, but was a significant source of DR error.

---

### 1.4 Dead Reckoning (The Core Navigation Method)

**What it is**: Starting from a known position (last fix), add vectors of (speed × time × heading) to estimate current position.

**The navigator's procedure**:
1. Every watch (4 hours), log the reading and compass heading in the log book
2. At each noon sight, update latitude precisely
3. Between noon sights (during overcast stretches, at night), run the DR forward

**Error accumulation**:
- Speed error ~15% → 15 NM error per 100 NM traveled
- Heading error ~3° over 1,000 NM → ~52 NM lateral error
- Leeway estimation error ~3° over 1,000 NM → another ~52 NM
- Current (not measured) → up to 150 NM error on transatlantic crossing (Gulf Stream)
- **After 3,000 NM transatlantic crossing**: DR position could be **150–300 NM** off in longitude, only 30–60 NM off in latitude (the latitude being reset daily by noon sight, the longitude accumulating unchecked)

**Source**: Wikipedia, "Dead Reckoning": "these errors tend to compound themselves over greater distances, making dead reckoning a difficult method of navigation for longer journeys."

---

### 1.5 Sounding Line (Lead and Line)

**Construction**: A lead weight (7–14 lbs for hand lead; 28–56 lbs for deep-sea lead) on a marked line. The bottom of the lead had a concavity filled with tallow to bring up bottom samples.

**Line markings**: Standard marks at 2, 3, 5, 7, 10, 13, 15, 17, 20 fathoms (cloth, leather, string of different materials for feel in darkness). Beyond 20 fathoms: single knot at 25, double at 30, etc.

**Uses**:
1. **Safety**: Avoid running aground in shallow water. Ships entering port would sound continuously.
2. **Position inference**: The tallow brought up sand, mud, shells, gravel, or came up clean (rock). Charts noted bottom composition. An experienced navigator could confirm position: "sandy bottom at 25 fathoms — we're entering the Florida Straits" or "shell bottom at 40 fathoms — northwest of Cuba."
3. **Depth contours**: Following a specific depth line (~20 fathom shelf edge) was a navigation strategy along coastlines.

**Practical limits**: Only useful in depths under ~100–120 fathoms (200m) with a hand lead; deep-sea lead could reach 300–400 fathoms but required all hands to haul back in, was slow, and could only be used while stopped or nearly stopped.

---

## 2. WHAT COULD THEY DETERMINE ACCURATELY?

### 2.1 Latitude: GOOD (within 15–60 NM)

| Instrument | Accuracy | Conditions |
|---|---|---|
| Backstaff (Davis Quadrant), skilled | ±0.25°–0.5° (~15–30 NM) | Calm seas, clear sky, noon sun or Polaris |
| Backstaff, practical at sea | ±0.5°–1° (~30–60 NM) | Moderate swells, some motion |
| Cross-staff (older, pre-1620) | ±1°–2° (~60–120 NM) | Increasing error toward horizon or sun |
| Mariner's astrolabe | ±1°–3° (~60–180 NM) | Sea state dependent, largely obsolete 1620+ |

**Key fact**: Latitude could be reset *every day at noon* on any clear day. After 3 weeks at sea, a navigator who had 10 clear noon sights was never more than 60 NM off in latitude. Overcast stretches (say, 5 days of clouds) would let latitude DR drift — add ~30 NM/day of potential latitude error in those periods.

**Polaris for latitude**: At night with clear skies, Polaris gives a direct latitude read (with a small correction for its ~0.7° offset from true celestial north, known from tables). This was the backup when noon sun wasn't available.

---

### 2.2 Longitude: VERY POOR (±60–600+ NM)

**The core problem**: Longitude requires knowing *exact time* at a reference meridian while simultaneously knowing local time (from the sun's position). Without a reliable timekeeper, there was no direct longitude method available to ordinary navigators.

**Methods available in 1600–1720**:

1. **Dead reckoning** (primary method): Start from a known longitude (e.g., Cadiz, or Barbados), count days × estimated speed × direction. Error grows continuously. After a transatlantic crossing (~3,500 NM): typical error **±150–300 NM** in longitude, with extreme cases (cloudy weather, bad currents, poor instruments) reaching **±500+ NM**.

2. **Lunar eclipses**: In theory, the time of a lunar eclipse was predictable and the same for all observers on Earth; comparing your local time of the event to the predicted time at a reference meridian gave longitude. In practice: (a) eclipses happen rarely, (b) you needed an accurate pendulum clock (which didn't work on ships), (c) Columbus tried this in 1494 and 1504 and got errors of **13° and 38°** — i.e., 780 and 2,280 NM. Not useful for navigation.

3. **Lunar distance method**: Measuring the angle between the moon and specific stars. Theoretically workable but computationally complex and required accurate lunar tables (Flamsteed's *Historia Coelestis*, 1712 — barely in our period) and a precision instrument. Not practically used by typical navigators in 1600–1720. It became more viable after 1767 (publication of the *Nautical Almanac*).

4. **Jupiter's moons** (Galileo's method, 1612): Eclipses of Jupiter's moons could serve as a universal clock. Accurate on land. **Completely impractical at sea** — the telescope couldn't be steadied on a ship's deck.

5. **Known departure + running time**: Most practical navigators simply noted their departure longitude (from a well-charted port), kept careful DR, and accepted the accumulating error.

**Columbus benchmark**: Wikipedia notes "longitude errors of 13 and 38° W respectively" on Columbus's two attempts using eclipse timing (1494, 1504). This is not unusual — Portuguese and Spanish longitude measurements 1514–1627 had "errors ranging from 2° to 25°" (Randles, 1985, cited in Wikipedia "History of Longitude").

**Simulation implications**: After a transatlantic passage, a ship's **believed longitude** should have an error sampled from roughly:
- Best case (favorable winds, dead-calm reading weather, experienced navigator): **±60–100 NM** longitude error
- Typical case: **±150–250 NM** longitude error  
- Poor conditions / less skilled: **±300–500 NM** longitude error
- Catastrophic (storms, weeks of overcast, bad instruments): **±500+ NM**

Latitude error is much smaller: ±30–60 NM normally, ±100 NM worst case.

---

### 2.3 Speed/Distance: MODERATE (±10–20%)

- Chip log: ±10–20% on speed through water
- Leeway not captured: adds another ~5–15% to distance-made-good error
- Current not captured: most significant error — the Gulf Stream runs at 2–4 knots; an unaware ship crossing it could have a 30–50 NM/day positional error just from current
- **Distance run after a 2-week passage**: expect ±15–25% error on total distance made good

---

### 2.4 Landmark Position: EXCELLENT (within visual range)

- **Masthead lookout height**: A lookout at a 100-ft mast sees the horizon at approximately **12 NM** (geometry: √(2 × height_ft × Earth_radius_NM / 5280)). Another ship's 100-ft mast appears over the horizon at ~12 NM from both sides, giving a mutual sighting range of ~24 NM. In practice, the peak of a mountain visible above that (say, the Blue Mountains of Jamaica at 7,402 ft) could be seen from **60–100 NM** in good weather.
- **Island landfall**: A flat island like Barbados (~100m peak) is visible ~20–25 NM. Dominica's volcanic peaks (1,447m) are visible from **100+ NM** in clear Caribbean weather.
- **Using high ground for approach**: Navigators explicitly used high peaks as waypoints. The Pico de Teide, Tenerife (3,718m) is visible from **~200 NM** — the standard European point of departure for transatlantic trade wind crossings; its bearing on departure was the last known longitude fix for the outbound voyage.
- **Low-lying danger** (sandbars, reefs): The Bahamas, Florida Keys, Grand Cayman — often not visible until you're in 3–5 fathoms. Sounding was the primary warning here, combined with lookouts at the bow and cathead for breakers (white water on surf visible **3–5 NM** in daylight, nearly invisible at night).

---

## 3. ROUTING STRATEGIES

### 3.1 Latitude Sailing (Running Down the Latitude)

**The dominant strategy for open-ocean navigation to a known destination.**

**Method**: 
1. Sail to the latitude of your destination (easy to verify with noon sight)
2. Turn east or west and sail along that latitude until you make landfall

**Example — Spain to Barbados** (lat ~13°N):
- Sail south from Cadiz (~36°N) to ~13°N while heading roughly SW to catch the trade winds
- Once at 13°N, turn west and run with the trades until Barbados appears on the bow
- The latitude is continuously confirmed by noon sight
- No longitude knowledge needed — you just keep sailing west until you hit something

**Example — Spain to Havana** (lat ~23°N):
- Sail to 23°N, then run west
- Pass north of the Antilles chain
- The Cuban coastline will appear ahead

**Why this worked so well**: In the Atlantic with trade winds, running west along a latitude was fast and reliable. Any overshoot brought you to the Caribbean island chain, which runs roughly north-south; you'd hit an island and ask local fishermen (or recognize the coast). Any undershoot meant you'd see Cuba/Hispaniola and know you were too far north or too far south.

**The danger**: Lee shores. If your latitude fix was wrong by 1° and you thought you were north of Cuba when you were at Cuba's latitude, you'd sail into Cuba at night. This happened. The number of Spanish ships wrecked on the Florida Keys and Bahama Banks is testament to latitude errors combined with dangerous low-lying coasts.

---

### 3.2 Trade Wind Exploitation

The trade winds were not just exploited — they *defined* the routes. By 1600, experienced pilots had documented these routes precisely.

**North Atlantic Subtropical High ("Azores High")**: Creates a clockwise wind gyre:
- **Outbound (Europe → Caribbean)**: Sail south along the African coast to ~15–20°N, pick up NE trades, then run west with them to the Caribbean at ~8–15 knots sustained
- **Return (Caribbean → Europe)**: Go north past the Bahamas into the Gulf Stream, ride it northeast to ~35–40°N, then pick up the SW Westerlies for Europe

**Trade wind belt characteristics**:
- Location: ~5–30°N (seasonal variation: further north in Northern Hemisphere summer)
- Direction: Steady NE to E in winter; more E to SE at the southern margins
- Speed: **10–20 knots** typically in winter (prime sailing season); lighter and variable in summer
- Reliability: Exceptional. Captains could count on NE trades for 10+ consecutive days

**"Horse Latitudes"** (~28–33°N): The subtropical high itself — area of light, variable winds and calms. Ships caught here could be becalmed for days. The famous story of horses being thrown overboard from Spanish ships to save fresh water may derive from this.

**Doldrums** (ITCZ, ~5°N–5°S): Light, variable winds, squalls, calms. Crossing the equator was feared partly because of this. Not directly relevant to Caribbean navigation unless heading to Brazil.

**Source**: Wikipedia, "Trade winds": "The Portuguese recognized the importance of the trade winds (*volta do mar*) in navigation in both the north and south Atlantic Ocean as early as the 15th century... Captain of a sailing ship seeks a course along which the winds can be expected to blow in the direction of travel."

---

### 3.3 The Spanish Treasure Fleet Routes (Documented Historical Routes)

The *Flota de Indias* system codified the Atlantic routes by the 1560s. These routes are the most precisely documented ocean navigation routes of the era.

**OUTBOUND (Spain → Americas)**:
1. Depart Cádiz/Seville (late spring, to avoid hurricane season in Caribbean)
2. Stop Canary Islands (La Gomera or Gran Canaria) for fresh water and provisions — **last longitude fix**
3. Pick up NE trade winds at ~20–25°N (Canaries are at 28°N) and head SW
4. Aim for latitude of destination: 
   - *Flota* (New Spain fleet): crosses at ~15–16°N to enter Caribbean via Dominica/Martinique, then sails west to Veracruz
   - *Galeones* (Tierra Firme fleet): crosses at ~10–13°N to enter via Trinidad/Tobago, then runs along the Main to Cartagena/Portobelo
5. Total crossing: ~3–4 weeks (Atlantic proper, after Canaries)

**RETURN (Americas → Spain)**:
1. All ships rendezvous at Havana (mandatory — lone ships were too vulnerable)
2. Exit Havana via Straits of Florida (76–80°W) — deliberately uses Gulf Stream
3. Gulf Stream carries them NE at 2–4 knots bonus speed to ~28–30°N
4. Exit Gulf Stream, continue NE to Azores (~38°N, 28°W) — another water/provision stop
5. Continue E to Cádiz with the SW Westerlies
6. Total voyage: 6–8 weeks from Havana to Cádiz

**Timing requirements**: The fleet had to leave Havana by **early July at the latest** to miss the peak hurricane season (August–October). This was not just preference — it was law. Late-departing ships regularly met with hurricanes.

**Source**: Wikipedia, "Spanish Treasure Fleet": "from the Spanish ports of Seville or Cádiz, the two fleets bound for the Americas sailed together down the coast of Africa, and stopped at the Spanish territory of the Canary Islands for provisions before the voyage across the Atlantic... both fleets sailed for Havana, Cuba, to rendezvous for the journey back to Spain."

---

### 3.4 The Gulf Stream (Homeward Route)

The Gulf Stream flows north through the Straits of Florida at **2–4 knots** (peak 4–6 knots in the core), then turns NE off Cape Hatteras, heading toward Europe.

- Width in Florida Straits: ~80 km (43 NM)
- Identifiable by: significantly warmer water temperature (surface), distinctive indigo-blue color (vs. green offshore water), **2–4 kt current** boosting the ship's speed
- **European discovery**: Ponce de León (1512) — his log notes "a current such that, although they had great wind, they could not proceed forward, but backwards" — the first documented encounter with the Stream's power
- **British ignorance**: As late as 1768, British mail packet ships were taking weeks longer than American merchants for England→New York because they didn't know to exploit or avoid the Stream. Benjamin Franklin had to explain it to them using his cousin Timothy Folger's whaler's knowledge.
- **Navigation implication**: A ship heading from the Caribbean to Europe in the Straits of Florida would receive a **2–4 knot bonus** — roughly adding 50–100 NM/day to speed made good. This was well-known to Spanish pilots by the mid-1500s.

---

### 3.5 Caribbean Inter-Island Navigation: Visual Pilotage Dominates

In the Caribbean itself, the longitude problem was largely irrelevant because:
- Islands are spaced 50–200 NM apart
- Islands are generally visible from 15–80 NM (depending on elevation)
- Known landmarks (mountain profiles, headland shapes) were documented in *ruttiers*
- Short passages could be completed in 1–3 days with limited DR accumulation

**The island-to-island procedure**:
1. Identify your current island (by visual recognition of peaks and headlands)
2. Know the approximate bearing and distance to next island from pilot book
3. Set off on that bearing
4. Watch for first sighting of destination peaks
5. Correct course based on actual appearance
6. Feeling way into the harbor by sounding

**Inter-island distances for context**:
- Barbados → St. Lucia: ~100 NM (typically 1.5–2 days with trades)
- St. Lucia → St. Kitts (via chain): ~250 NM total (3–4 days sailing each hop)
- Jamaica → Cuba (nearest point): ~90 NM
- Cuba → Hispaniola (eastern tip): ~50 NM at narrowest
- Trinidad → Tobago: ~20 NM (visible on a clear day from sea level)

**Windward vs. Leeward Islands problem**: The Lesser Antilles are oriented N-S while the trade winds blow E-W. Sailing northward *through* the island chain against the trades was very difficult. Most traffic beat upwind along the island chain by hopping from one anchorage to the next, gaining small amounts on each tack. The preferred route was to go well south of the chain, then tack north from open ocean.

---

### 3.6 Cabotage (Coastal Sailing) vs. Open Ocean

**Cabotage**: Sailing within sight or near-proximity of the coastline. Dominant navigation method for any coast where charts were available.

**Procedure**:
- Keep headlands visible (or soundings in range) at all times
- Use prominent landmarks for cross-bearings to establish position
- Anchor at night in unfamiliar waters (essential — navigating on a lee shore at night was suicide)
- Use local pilots for harbor approaches

**The "anchor at night" rule**: Until approximately 1700 (when better charts appeared), experienced captains would anchor at dusk if approaching an unknown coast. Morning light was used to identify position before proceeding. Disregarding this rule was a major cause of wrecks.

**Where pilots were hired**: 
- Every major port required boarding a local harbor pilot
- Havana: pilots were mandatory for all large ships; the entrance channel between Morro and La Punta was only ~200m wide
- Portobelo: the harbor entrance required local knowledge of the reefs
- Veracruz: extremely dangerous approaches with the San Juan de Ulúa reef; the local *piloto* was essential
- Cartagena: two channels (Bocagrande/Bocachica), silting conditions changed seasonally
- New Providence (Bahamas): extremely shallow — ships needing to call there hired Bahamian fishermen as pilots who knew each channel by memory

---

## 4. HAZARD AVOIDANCE

### 4.1 The Standard "Approach to Land" Procedure

When expecting land (after a passage), the careful navigator:

1. **Goes to the mast-head** himself or posts the best lookout (higher = more range)
2. **Heaves-to at dusk** if land not yet sighted but expected — wait for daylight
3. **Starts sounding** at the edge of the continental shelf (generally when depths come up from >200 fathoms to <100 fathoms)
4. **Slows down** — reduces canvas to topsails only in uncertain waters
5. **Posts bow watch** — two men on the bowsprit or at the catheads watching for surf/breakers
6. **Checks bottom samples** from the tallow-armed lead — matches expected seabed composition from pilot book or rutter

**Soundings as position indicator**:
- Florida east coast shelf: depths of 20 fathoms run ~10–15 NM offshore; shelf break drops sharply
- Cuba north coast: shelf narrow; drop to 100+ fathoms within 5–10 NM
- Grand Bahama Bank: extensive shallow (<10 fathom) water visible by color change (turquoise vs. deep blue)
- Bahama color change was well-known: clear blue→pale turquoise meant "shoal water ahead" — could be seen from the mast-head before soundings were needed

### 4.2 Lee Shores — The Sailor's Greatest Fear

A **lee shore** is land downwind from the ship. With wind pushing the ship toward the coast, escape requires beating upwind — which square-rigged ships do poorly.

**The physics**: A typical square-rigger could make 5–6° to windward of the true wind (effectively sailing about 70–75° off the wind, not 45° like a modern yacht). In a storm with 30+ kt winds, the ship could be making **3–4 knots leeway** even at full-speed. If the coast was 10 NM to leeward, they might have 2–3 hours at most before disaster.

**Historical examples**:
- The 1715 Spanish Treasure Fleet: 10 ships wrecked on the Florida coast during a hurricane. Navigating the Gulf Stream route (close to the Florida shore) when caught by a north-moving hurricane meant no sea room to windward. ~1,000 men died.
- Sir Francis Drake's fleet caught on the Cuban coast in a hurricane (1591): lost several ships
- The Shoals of Abaco (Bahamas): countless Spanish ships wrecked on the northeast Bahama banks when longitude errors placed them further west than they were

**Navigator response options to a lee shore**:
1. **Anchor** (if bottom was catchable and anchor held — not always in deep water)
2. **Wear ship** (turn downwind through 270° rather than upwind through 90° — much easier but costs sea room)
3. **Make sail and try to claw off** (beating upwind — only possible if not too far in)
4. **Cut anchors, run for it, and hope** (in the worst case)

### 4.3 Common Causes of Navigational Disaster

In roughly descending frequency:
1. **Longitude error leading to early landfall**: Thought they were 200 NM from land; they were 50 NM.
2. **Reef and shoal not on charts**: Much of the Caribbean was poorly charted. New reefs were discovered the hard way.
3. **Current drift**: Gulf Stream or Caribbean Current pushing ship unexpectedly
4. **Fog/squall reducing visibility to zero** at critical moment
5. **Navigational complacency**: After weeks at sea, exhausted sailors relaxed near known coasts — many wrecks happened within sight of destination
6. **Hurricane**: No advance warning whatsoever; barometric pressure instruments (barometers) not in use until late 17th century, and their application to storm prediction only developed later

---

## 5. CHARTS, RUTTERS, AND NAVIGATIONAL KNOWLEDGE

### 5.1 Available Chart Types (1600–1720)

**Portolan-style charts**:
- Manuscript charts on vellum, with rhumb-line networks (windrose lines) from multiple compass roses
- Excellent for the Mediterranean; carried into Atlantic use but poorly adapted for the Caribbean
- Did not use latitude/longitude grids; instead used compass bearings and estimated distances
- Caribbean portolans were largely inaccurate copies of copies, with distorted coastlines
- **Not Mercator projection** — straight rhumb lines on a portolan chart are only accurate for short distances (good Mediterranean practice failed at Atlantic scales)
- **Source**: Wikipedia, "Portolan Chart": "They are manuscript charts rendered using ink on vellum sheets... a content focus on coastal regions, networks of colour-coded straight lines... place names inscribed perpendicular to the coastline contours."

**Early Mercator projection charts**:
- Gerardus Mercator published his famous world map in 1569; the projection became standard for nautical use *slowly* — not widely adopted for practical navigation until late 17th century
- **Key property**: Rhumb lines (constant compass bearing courses) are straight lines on a Mercator chart — perfect for dead reckoning navigation
- By 1600, some English navigators had Mercator charts; by 1650, Mercator charts were becoming standard for ocean passages
- Caribbean Mercator charts of the era: longitude distortions were common because no one had accurately determined the longitudes of Caribbean ports. Charts could be wrong by **1–3°** (60–180 NM) in longitude placement of islands and coasts.

**Waggoners (Waghaenær charts)**:
- Lucas Janszoon Waghenaer published *Spiegel der Zeevaerdt* (1584) — the first comprehensive printed nautical atlas for European coasts
- English translation "The Mariner's Mirror" (1588) became standard on English ships
- For the Caribbean: equivalent **English West Indies rutters** were circulated in manuscript; Spanish *derroteros* (pilot books) were jealously guarded state secrets
- Dutch *waggoners* extended coverage to Caribbean by the 1620s

**Rutters (Ruters/Derroteros)**:
- Not charts — written sailing directions. "From X, steer SW by S for 3 days until the latitude of Y, then steer W until you sight the north coast..."
- Often included: prevailing winds for each month, expected currents, harbor approach bearings, anchorage depths, landmarks to recognize, dangerous shoals
- **The Spanish monopoly**: Spanish *derroteros* of the Caribbean were classified. Captured Spanish pilots or their books were extremely valuable to enemies. When Drake captured Spanish ships, he kept any charts and pilots he found.
- **Waggoners were pirated**: English pirate editions of Dutch waggoners circulated widely. The piracy of navigation information was as important as piracy of treasure.

### 5.2 Accuracy of Period Charts

**Caribbean chart errors, c.1600–1720**:
- Latitude of major ports: typically accurate to **±0.5°** (30 NM) in good charts; ±1–2° in poor charts
- Longitude of Caribbean ports vs. Spain: typical error **±2–4°** (120–240 NM) — charts placed the Caribbean too far east or west depending on who drew them
- Small islands, reefs: often wrong by 20–50 NM in position, or simply absent
- Coastline detail: acceptable for major ports; poorly surveyed for minor coasts

**Notable chart disasters**:
- The entire Caribbean was about **3–5° too far east** on many early 17th-century charts (a systematic error from imprecise longitude determination). This caused navigators to overshoot islands they expected and find them unexpectedly.
- The Bermuda Islands were so consistently misplotted that the Virginia Company's 1609 fleet ran right into them (founding the first English colony there by accident).

### 5.3 Knowledge Transmission

**Pilot books and oral tradition**: Most navigational knowledge was transmitted through:
1. **Apprenticeship** — young boys sailed as cabin boys/pages, absorbing practical skill over years
2. **Pilot books** (rutters): Written directions, tide tables, anchorage notes. The experienced pilot's accumulated knowledge in printed/manuscript form.
3. **Examination systems**: Spain's *Casa de Contratación* (founded 1503) required examination of pilots before certification for the Indies routes. This created a body of standardized knowledge.
4. **Captured knowledge**: Captured Spanish pilots (and their books) were worth a fortune. Drake, Hawkins, and other privateers specifically targeted Spanish navigational experts.

**The "Pilot" as profession**: 
- Harbor pilots (practical men who knew local waters) were a distinct profession from ocean navigators
- Major ports had licensed pilots who boarded all ships at the harbor entrance
- Ocean pilots (*pilotos*) were highly paid specialists — the senior navigation officer on a Spanish galleon could earn as much as the captain
- Pirate crews often included former merchant or naval navigators; having a "good pilot" was cited as critical to successful pirate voyages

**Halley's chart (1700)**: Edmond Halley published the first magnetic declination chart for the Atlantic in 1700 — a landmark in navigational science that partially addressed the compass correction problem. But its distribution was uneven; provincial captains might not have seen it for years.

---

## 6. SIMULATION IMPLEMENTATION MODEL

### 6.1 Navigation State (What a Ship Knows)

```
struct NavigationState {
    // What the ship THINKS its position is (may differ from actual)
    believed_position: Vec2,         // lat/lon in NM coordinates
    
    // Uncertainty envelope (error bounds)
    latitude_error_nm: f32,          // typically 15–60 NM
    longitude_error_nm: f32,         // typically 60–300 NM; grows with time at sea
    
    // DR inputs (contributing to error accumulation)
    days_since_last_fix: f32,        // grows uncertainty
    last_fix_quality: FixQuality,    // Celestial, Landmark, Sounding, DR_only
    
    // Known waypoints/targets
    destination: Option<PortId>,
    known_route: Option<RouteId>,
    
    // Current navigational strategy
    strategy: NavStrategy,
}

enum NavStrategy {
    LatitudeSailing { target_lat: f32, run_direction: EastWest },
    CoastalPilotage { following_coastline: CoastId },
    IslandHopping { next_island: PortId, bearing: f32 },
    DeadReckoning { target_bearing: f32 },
    SeekingLandfall,      // knows approximate direction, watching for land
}

enum FixQuality {
    Celestial,        // noon sight, clear sky: latitude ±30 NM, lon unchanged
    LandmarkVisual,   // actual sighting of known coast/island: full reset ±5 NM
    Sounding,         // depth/bottom matches chart: partial fix ±20 NM
    DeadReckoning,    // no fix: error grows
}
```

### 6.2 Error Model (Simulation Parameters)

**At departure from a known port**: 
- `latitude_error = 0` (position is perfectly known)
- `longitude_error = 0`

**Each sim day at sea (dead reckoning)**:
```
latitude_error  += 3–5 NM/day  (compass + leeway; partially offset by noon sights)
longitude_error += 10–15 NM/day (speed error + compass + current)
```

**On a noon sight (clear day)**:
```
latitude_error = min(latitude_error, 30 NM)  // reset to 30 NM best-case
// longitude_error UNCHANGED — noon sight gives latitude only
```

**On landmark sighting** (coast/island identified within 15 NM):
```
latitude_error  = 5 NM   // full reset
longitude_error = 5 NM   // full reset
```

**On sounding confirmation** (depth/bottom matches chart):
```
latitude_error  = min(latitude_error, 20 NM)  // partial reset
longitude_error = min(longitude_error, 40 NM)  // partial longitude reset from chart
```

**Practical caps**:
- Latitude error rarely exceeds **60–80 NM** because noon sights are taken whenever possible
- Longitude error can grow to **300+ NM** on long transatlantic passages with cloudy weather

### 6.3 Landmark Visibility Model

| Object | Height | Visibility (from masthead ~100ft) |
|---|---|---|
| Low-lying flat island (Barbados) | ~300 ft | ~30 NM |
| Typical Caribbean peak (St. Lucia) | ~3,000 ft | ~80 NM |
| High volcanic peaks (Dominica, Guadeloupe) | ~5,000 ft | ~100 NM |
| Cuban coastal hills | ~300–1,000 ft | ~40–60 NM |
| Pico de Tenerife (departure reference) | 12,200 ft | ~200 NM |
| Flat coastline/beach (Florida) | ~20 ft | ~15–18 NM |
| Surf/breakers on reef | ~5 ft | ~3–5 NM |

**Formula** (approximate, for simulation):
```
visibility_nm = sqrt(height_masthead_ft / 0.67) + sqrt(height_object_ft / 0.67)
```
(This gives the sum of the two horizon distances)

### 6.4 Route Decision Rules (Ship AI)

**For a ship going from Port A to Port B**:

1. **If known established route exists** (e.g., Cadiz→Cartagena treasure fleet route):
   - Follow the documented waypoints (Canaries → cross at 12°N → steer for Cartagena)
   - Do not "A* pathfind" — follow the cultural/wind-determined route

2. **If inter-Caribbean short hop** (under 300 NM):
   - Navigate by bearing to known island + landmark watch
   - latitude sailing not needed — dead reckoning is accurate enough for 2-day passages
   - Arrive at night? Heave-to and wait for dawn

3. **If open-ocean passage** (300+ NM):
   - Find the latitude of the destination first (sail N or S until noon sight confirms target lat)
   - Then run E or W along that latitude
   - Keep watching for landmark/soundings to reset error

4. **If lost** (error exceeds threshold — say longitude_error > 200 NM):
   - "Seek easting/westing to known longitude" — sail toward nearest charted coast
   - Use soundings on approach
   - Slow down, post lookouts

5. **If near known hazard** (shallow bank, reef):
   - Station leadsman continuously
   - Speed reduced to minimum steerage
   - Post masthead lookout
   - Anchor at dusk if in doubt

### 6.5 The Caribbean-Specific Navigation Context

In the Caribbean, the navigation model simplifies significantly because:

**The Lesser Antilles** form a "wall" running NS that a ship crossing the Atlantic will hit if it's aimed right. Once you identify which island you've hit (they're distinctive), you have a perfect fix. Then:
- You're within 1–3 days of most other islands
- Visual piloting takes over entirely
- DR is only needed for the inter-island hops

**The Greater Antilles** (Cuba, Hispaniola, Jamaica, Puerto Rico) have large coastlines with distinctive profiles. Experienced Caribbean pilots could identify specific capes and mountains by profile and bearing alone.

**Specific Jamaica→Cuba scenario** (~90 NM):
- Set off on a NW bearing from the north coast of Jamaica
- Sail for 12–18 hours (with trade winds at ~5–7 knots)
- Cuba's southern mountains (~800m) become visible from ~70 NM
- Adjust course to approach desired port (typically Santiago de Cuba or Havana)
- Use pilot for harbor entrance

**The "wrecking ground" problem** (Florida/Bahamas):
- The Florida Straits are ~80 NM wide but the eastern side has the Bahama Banks
- Banks are invisible at night and in swells — just shallow, turquoise, dead-flat water
- Ships heading from Jamaica northward to enter the Gulf Stream were threading a needle
- Standard procedure: hug the Florida coast (deep water within 5 NM) rather than risk the banks

---

## 7. KEY HISTORICAL CASES FOR SIMULATION CALIBRATION

**Case 1 — The 1715 Plate Fleet Hurricane**:
- 11 ships departing Havana July 1715, caught by hurricane July 31 off Cape Canaveral
- Strong evidence of longitude uncertainty: pilots placed themselves ~50 NM further east (offshore) than actual position
- Result: 10 of 11 ships wrecked on the Florida coast; ~1,000 dead, 14 million pesos in treasure
- **Simulation lesson**: Even expert navigators in familiar waters could be fatally off in longitude

**Case 2 — Columbus's longitude errors (1494, 1504)**:
- Using eclipse timing (the best available method): errors of **13° and 38°** longitude (780 and 2,280 NM)
- **Simulation lesson**: Longitude methods other than DR were essentially useless for working sailors

**Case 3 — Benjamin Franklin's Gulf Stream story (1768)**:
- British mail packets taking 2+ weeks *longer* than American merchantmen for England→New England
- British captains were fighting the Gulf Stream westbound without knowing it
- Nantucket whalers had been exploiting/avoiding the Stream for generations via practical observation (water color, temperature, whale behavior)
- **Simulation lesson**: Experienced local captains had tacit knowledge (current identification, weather patterns) that formal charts didn't capture

**Case 4 — Drake's Pacific charts (1578)**:
- Drake captured Spanish pilot Nuno da Silva specifically for his charts and navigational expertise
- Da Silva was kept aboard *Golden Hind* for over a year as a navigational asset
- **Simulation lesson**: A captured navigator was a strategic prize, not just a prisoner

---

## 8. SIMULATION PARAMETERS SUMMARY TABLE

| Parameter | Value | Source/Notes |
|---|---|---|
| Latitude accuracy (backstaff, good conditions) | ±0.25°–0.5° (±15–30 NM) | Backstaff article, instrument specs |
| Latitude accuracy (typical at sea) | ±0.5°–1° (±30–60 NM) | Sea motion, overcast |
| Longitude accuracy (DR, short passage <500 NM) | ±30–80 NM | Speed + compass errors |
| Longitude accuracy (transatlantic, 3,000+ NM) | ±150–300 NM typical | Historical wrecks, Randles 1985 |
| Longitude accuracy (bad conditions) | ±300–600+ NM | 1715 treasure fleet disaster |
| DR speed error (chip log) | ±10–20% | Sandglass + sag + current |
| Leeway (square-rigger beam reach) | 3–8° | Beam wind, standard |
| Compass error (uncorrected declination) | ±2–8° | Caribbean declination c.1700 |
| Land sighting (low island, 300ft) | ~30 NM | Geometry of 100ft mast |
| Land sighting (high peak, 3000ft) | ~80 NM | |
| Land sighting (Pico de Tenerife, 12000ft) | ~200 NM | Documented departure landmark |
| Breakers/surf visible | 3–5 NM | Daytime only |
| Standard latitude sailing transit speed | 5–8 knots in trade winds | Ship types research |
| Gulf Stream current boost | +2–4 knots | Wikipedia, Gulf Stream |
| Caribbean trade wind belt | 10–25 knots NE–E | Trade Winds article |
| Typical transatlantic crossing (Spain→Caribbean) | 3–4 weeks | Treasure fleet records |
| Typical Caribbean→Spain return | 6–8 weeks (longer, upwind elements) | Treasure fleet records |
| Daily latitude error drift (DR only) | +3–5 NM/day | Compass + leeway |
| Daily longitude error drift (DR only) | +10–15 NM/day | Speed + compass + current |
| Fix reset on landfall | ±5 NM both axes | Visual identification |
| Fix reset on noon sight | lat ±30 NM; lon unchanged | Celestial fix gives latitude only |

---

## 9. GAPS AND UNCERTAINTIES

1. **Exact DR error numbers** for this era are hard to pin down from Wikipedia-level sources. The ±150–300 NM transatlantic longitude error is well-documented historically (e.g., Randles' study), but per-day accumulation rates are engineering estimates based on instrument characteristics, not primary sources.

2. **Spanish chart accuracy** for Caribbean waters (c.1600–1660) is poorly documented in English sources. The *derroteros* were secret documents; published scholarship on their accuracy is limited.

3. **Chip log calibration** varied substantially between fleets and nations. The 7-fathom vs. 8-fathom vs. 47.25-foot standards were all in simultaneous use.

4. **Leeway estimation**: No primary sources give quantified accuracy of visual leeway estimates; the ±3–8° range is a physical estimate from square-rigger performance data in the ship-types research.

5. **Magnetic declination values for Caribbean c.1650** (pre-Halley): I've cited approximate values (~5–8°W) but these are extrapolated backward from Halley's 1700 chart. For a detailed simulation, the NOAA historical geomagnetic model (IGRF) has reconstructions back to ~1600 and should be consulted for per-location accuracy.

6. **Caribbean island pilot knowledge**: The "identifying islands by peak profile" was described in numerous contemporary accounts (Dampier, Exquemelin, de Lussan) but specific visibility ranges are not quantified in those primary sources — the geometry-based estimates here are engineering calculations.

**Suggested follow-up**: William Dampier's *A New Voyage Round the World* (1697) and *A Discourse of Trade Winds* (1699) contain the most detailed primary-source descriptions of Caribbean/Pacific navigation practice from a literate practitioner of the exact era. Dampier was a buccaneer-turned-navigator-turned-writer who recorded wind, current, and navigation practice systematically. His work is the equivalent of a primary source navigator's manual for the simulation period.

---

*Report based on: Wikipedia articles (Dead Reckoning, Backstaff, Cross-staff, Chip log, History of Longitude, Portolan Chart, Trade Winds, Gulf Stream, Celestial Navigation, Magnetic Declination, Spanish Treasure Fleet, Sounding Line); cross-referenced with project's existing map-and-geography.md, ship-types.md, and naval-combat.md research.*