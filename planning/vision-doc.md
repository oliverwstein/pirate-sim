# Pirate Sim: Vision & Target Feature Set
## The Living Caribbean, 1600–1720

## 1. Product Vision

**Pirate Sim** is a headless, deterministic, continuous-time simulation of the 17th and early 18th-century Caribbean. It is designed to model the intersection of physical geography, mercantilist economics, naval warfare, and human capital. 

Unlike *Sid Meier’s Pirates!*, which relies on random number generators and spawn tables to simulate a living world, this engine builds the world from the bottom up. If a port is starving, it is because ships carrying provisions were intercepted by privateers or delayed by storms. If a pirate fleet is massive, it is because bankrupt merchants with plummeting morale mutinied. Because the entire world is not modeled, the economies behind the ports and factions can provide something akin to those spawn tables when needed, but the crux of the model should be a balanced system. 

The ultimate goal is an engine capable of running autonomously as a mesmerizing "aquarium" of colonial history, while exposing a clean API for frontends (visualizers, games, analytical tools) to observe or possess actors within it.

---

## 2. The Tripartite AI Architecture

To achieve true emergent complexity, the simulation operates on three distinct layers of autonomous agency. Each layer is driven by its own **Behavior Tree (BT)**, responding to the layer above it and commanding the layer below it. Initially, only the ships need behavior trees; faction and port behavior can be modeled via AI later. A faction's decision space is reminiscent of that of a grand strategy game; a captain's decision space is akin to that of the player in Sid Meier's Pirates!; the decision space of a port is the most in doubt; it may not be necessary to model them as BTs. 

### 2.1 The Faction AI (The Metropole)
Factions (England, Spain, France, Netherlands, Free) evaluate macro-geopolitical state on a monthly/seasonal cadence.
*   **Diplomatic Stances:** Factions declare war, negotiate peace, and form alliances, altering the global `RelationsMatrix`.
*   **Mercantilist Policy:** Factions enforce Trade Laws (e.g., the English Navigation Acts, the Spanish *Casa de Contratación*). They can enact embargoes or grant special trading licenses (*Asiento*).
*   **Force Projection:** Factions allocate metropolitan budgets to dispatch Naval Squadrons (Ships of the Line and Frigates) to the Caribbean, or issue Letters of Marque to incentivize Privateers. If they are at war, they may send out fleets to attack or conquer islands and ports.

### 2.2 The Port AI (The Governor & Merchants)
Ports evaluate their local survival and prosperity on a daily/weekly cadence.
*   **Defense & Infrastructure:** The Port BT decides how to spend its treasury. Does it build Forts? Upgrade the Shipyard? Expand the careening wharves?
*   **Smuggling & Corruption:** If a port is starving due to a blockade or monopoly laws, the Governor's BT evaluates the risk/reward of accepting bribes to allow Dutch smugglers to dock.
*   **Local Bounties:** Ports terrorized by pirates can dip into their treasury to hire Pirate Hunters (merchants subsidized to adopt a combat policy).
*   **Other Policies TBD***

### 2.3 The Captain AI (The Ships)
Captains (the current `ShipAI`) evaluate their immediate surroundings on an hourly cadence.
*   **Navigators:** Captains do not know objective reality. They act on Dead Reckoning estimates, correct course via landmark fixes, and react to storms, lee shores, and fouled hulls.
*   **Traders:** Captains evaluate multi-hop triangular trade routes, balancing provisions, cargo space, and the risk of capture.
*   **Combatants:** Based on their `ShipPolicy` (Merchant, Privateer, Pirate, Navy, Pirate Hunter), captains evaluate targets using the spatial hash, weighing relative speed, firepower, and faction alignment before choosing to pursue, flee, or patrol.

---

## 3. Core Simulation Elements

To meet the complexity of *Pirates!* (and surpass its economic depth), the simulation must realize the following interwoven systems.

### 3.0 Cellular Automata.
The behavior of ships should work as cellular automata without rigid coordination. The Spanish Treasure Fleet may have a unique BT, but each ship in that fleet should move based on its own BT. This would be especially relevant for avoiding battle/detection and grouping for defense. One merchant ship may flee a pirate; if there are three merchant ships of the same faction all together, they should be able to band together. 

### 3.1 Naval Combat & Tactics (Double-Buffered CA)
Combat is not a localized minigame; it occurs directly on the world map via the Cellular Automata command pipeline. 
*   **The Weather Gauge:** Wind direction dictates engagement terms. Windward ships choose the range; leeward ships suffer speed penalties and expose their lower hulls.
*   **Ordnance & Gunnery:** Cannons are physical cargo (`GoodId::Ordnance`). Ships fire broadsides requiring Powder and Shot. The BT chooses shot types: Round shot (hull damage), Chain shot (rigging/speed damage), or Grapeshot (crew slaughter prior to boarding).
*   **Boarding Melees:** Ships closing to <0.05 NM grapple. Success is driven by crew ratios, veteran status (`crew_seasoned`), and morale.
*   **Prizes & Captures:** Victorious captains can strip cargo, ransom officers, burn the hull, or split their crew to sail the prize back to a friendly port for Admiralty Court adjudication (massive silver influx).

### 3.2 Land Warfare & Port Sieges
Ports (and islands) should be sackable and conquerable by hostile factions and, in special cases, possibly even by a pirate fleet. There should not be pirate ports by default; they must be formed by events or conquered. 
*   **Fortifications vs. Ships:** Ports build shore batteries. Forts have massive accuracy and armor advantages over wooden ships, requiring attacking fleets to possess 5:1 gun superiority to force a harbor.
*   **Amphibious Assaults:** Fleets can land crews outside the harbor zone to attack cities from the landward side (the Henry Morgan maneuver), bypassing harbor forts but risking disease and militia resistance.
*   **Sacking & Ransoms:** A conquered port has its treasury looted, stockpiles stripped, and buildings downgraded. The attacking faction can hold the port for ransom or formally annex it (shifting its Faction flag).

### 3.3 The Deep Economy & Logistics
The economy goes beyond simple buy-low/sell-high mechanics.
*   **The Commodity Web:** 20+ distinct goods. Sugar, Tobacco, and Logwood flow out; Manufactures, Ordnance, and Enslaved Persons flow in; Provisions and Salt circulate locally.
*   **Ship Maintenance:** Hulls accumulate *Teredo* worm damage and biofouling. Ships must periodically find safe beaches or port wharves to careen, taking them out of action for weeks.
*   **Shipbuilding:** Hulls require Naval Stores, Manufactures, Timber, and Silver to construct. A shortage of pitch and tar limits a faction's ability to replace lost ships.

### 3.4 Human Capital & Demographics
Ships are useless without sailors.
*   **The Sailor Pool:** Ports generate seasoned and unseasoned sailors based on their category (European Hub vs. Pirate Haven).
*   **Disease & Attrition:** Unseasoned European sailors face horrific mortality rates ("the seasoning") during their first year in the Caribbean.
*   **Wages & Morale:** Crews demand wages and food. Starvation, crushing debt, and unpaid wages tank morale.
*   **The Mutiny Pipeline:** When merchant morale collapses, the crew murders the officers, zeros out the ship's debt, flips the flag to `ShipPolicy::Pirate`, and elects a new captain.

### 3.5 The "Famous Pirate" Subsystem
To capture the flavor of historical legends (Blackbeard, Roberts, Morgan):
*   **Reputation & Infamy:** Captains who successfully capture prizes accrue "Infamy". High Infamy grants bonuses to intimidation (merchants surrender without firing) and recruitment (sailors flock to successful pirates).
*   **Named Entities:** While standard ships are generic, the engine supports tagging specific `ShipId`s with persistent named Captains, allowing frontends to track the careers, bounties, and eventual demises of legendary actors.
*   **Pirate Havens & Republics:** High-infamy pirates naturally congregate at poorly defended, high-corruption ports (e.g., Nassau, Tortuga), dynamically forming "Pirate Republics" that Faction Navies will eventually be forced to blockade and clear out.

---

## 4. Technical Guarantees

To ensure the simulation remains scalable and historically rigorous:

1.  **Strict Determinism:** Given the same Random Seed and the same starting RON datasets, a 10-year headless simulation will yield the exact same ship coordinates, market prices, and cannonball strikes every time.
2.  **Order-Independent Evaluation:** The adoption of a strict Read-Compute-Write (Double Buffering) pipeline ensures that AI decisions and combat outcomes do not depend on the memory layout or iteration order of the `SlotMap`.
3.  **Data-Oriented Scalability:** The engine avoids deep object-oriented inheritance. Systems iterate over flat arrays and contiguous memory blocks, allowing the simulation to effortlessly handle 1,000+ active ships and 50+ ports on a single thread, leaving ample overhead for visualization or frontend logic.