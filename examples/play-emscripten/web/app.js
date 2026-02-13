// Backend wrapper functions
const backend = {
	start() {
		ccall("start", null, [], []);
	},
	stop() {
		ccall("stop", null, [], []);
	},
	getCpuLoad() {
		return ccall("get_cpu_load", "number", [], []);
	},
	setMetronomeEnabled(enabled) {
		ccall("set_metronome_enabled", null, ["boolean"], [enabled]);
	},
	setActiveSynth(synthType) {
		ccall("set_active_synth", null, ["number"], [synthType]);
	},
	setSynthVoiceCount(voiceCount) {
		ccall("set_synth_voice_count", null, ["number"], [voiceCount]);
	},
	getSynthParameters() {
		const paramsPtr = ccall("get_synth_parameters", "number", [], []);
		if (paramsPtr !== 0) {
			const paramsJson = UTF8ToString(paramsPtr);
			const params = JSON.parse(paramsJson);
			ccall("free_cstring", null, ["number"], [paramsPtr]);
			return params;
		}
		return null;
	},
	setSynthParameterValue(paramId, normalizedValue) {
		ccall(
			"set_synth_parameter_value",
			null,
			["number", "number"],
			[paramId, normalizedValue],
		);
	},
	synthParameterValueToString(paramId, normalizedValue) {
		const valuePtr = ccall(
			"synth_parameter_value_to_string",
			"number",
			["number", "number"],
			[paramId, normalizedValue],
		);
		if (valuePtr !== 0) {
			const valueStr = UTF8ToString(valuePtr);
			ccall("free_cstring", null, ["number"], [valuePtr]);
			return valueStr;
		}
		return null;
	},
	synthParameterStringToValue(paramId, str) {
		const value = ccall(
			"synth_parameter_string_to_value",
			"number",
			["number", "string"],
			[paramId, str],
		);
		if (!Number.isNaN(value)) {
			return value;
		}
		return null;
	},
	synthNoteOn(key, velocity) {
		ccall("synth_note_on", null, ["number", "number"], [key, velocity]);
	},
	synthNoteOff(key) {
		ccall("synth_note_off", null, ["number"], [key]);
	},
	randomizeSynth() {
		const updatesPtr = ccall("randomize_synth", "number", [], []);
		if (updatesPtr !== 0) {
			const updatesJson = UTF8ToString(updatesPtr);
			const updates = JSON.parse(updatesJson);
			ccall("free_cstring", null, ["number"], [updatesPtr]);
			return updates;
		}
		return null;
	},
	getModulationSources() {
		const sourcesPtr = ccall("get_modulation_sources", "number", [], []);
		if (sourcesPtr !== 0) {
			const sourcesJson = UTF8ToString(sourcesPtr);
			const sources = JSON.parse(sourcesJson);
			ccall("free_cstring", null, ["number"], [sourcesPtr]);
			return sources;
		}
		return null;
	},
	getModulationTargets() {
		const targetsPtr = ccall("get_modulation_targets", "number", [], []);
		if (targetsPtr !== 0) {
			const targetsJson = UTF8ToString(targetsPtr);
			const targets = JSON.parse(targetsJson);
			ccall("free_cstring", null, ["number"], [targetsPtr]);
			return targets;
		}
		return null;
	},
	setModulation(sourceId, targetId, amount, bipolar) {
		return ccall(
			"set_modulation",
			"number",
			["number", "number", "number", "boolean"],
			[sourceId, targetId, amount, bipolar],
		);
	},
	clearModulation(sourceId, targetId) {
		return ccall(
			"clear_modulation",
			"number",
			["number", "number"],
			[sourceId, targetId],
		);
	},
	getAvailableEffects() {
		const effectsPtr = ccall("get_available_effects", "number", [], []);
		if (effectsPtr !== 0) {
			const effectsJson = UTF8ToString(effectsPtr);
			const availableEffects = JSON.parse(effectsJson);
			ccall("free_cstring", null, ["number"], [effectsPtr]);
			return availableEffects;
		}
		return null;
	},
	addEffect(effectName) {
		const jsonPtr = ccall("add_effect", "number", ["string"], [effectName]);
		if (jsonPtr !== 0) {
			const jsonStr = UTF8ToString(jsonPtr);
			const effectData = JSON.parse(jsonStr);
			ccall("free_cstring", null, ["number"], [jsonPtr]);
			return effectData;
		}
		return null;
	},
	removeEffect(effectId) {
		return ccall("remove_effect", "number", ["number"], [effectId]);
	},
	effectParameterValueToString(effectId, paramId, normalizedValue) {
		const valuePtr = ccall(
			"effect_parameter_value_to_string",
			"number",
			["number", "number", "number"],
			[effectId, paramId, normalizedValue],
		);
		if (valuePtr !== 0) {
			const valueStr = UTF8ToString(valuePtr);
			ccall("free_cstring", null, ["number"], [valuePtr]);
			return valueStr;
		}
		return null;
	},
	effectParameterStringToValue(effectId, paramId, str) {
		const value = ccall(
			"effect_parameter_string_to_value",
			"number",
			["number", "number", "string"],
			[effectId, paramId, str],
		);
		if (!Number.isNaN(value)) {
			return value;
		}
		return null;
	},
	setEffectParameterValue(effectId, paramId, normalizedValue) {
		ccall(
			"set_effect_parameter_value",
			null,
			["number", "number", "number"],
			[effectId, paramId, normalizedValue],
		);
	},
};

// CPU display
let cpuDisplayUpdater = null;

function updateCpuDisplay() {
	const cpuDisplayElement = document.getElementById("cpu-display");
	if (cpuDisplayElement) {
		const load = backend.getCpuLoad() * 100;
		cpuDisplayElement.textContent = `CPU Load: ${load.toFixed(2)}%`;
	}
	cpuDisplayUpdater = requestAnimationFrame(updateCpuDisplay);
}

function startCpuDisplay() {
	if (!cpuDisplayUpdater) {
		updateCpuDisplay();
	}
}

function stopCpuDisplay() {
	if (cpuDisplayUpdater) {
		cancelAnimationFrame(cpuDisplayUpdater);
		cpuDisplayUpdater = null;
		const cpuDisplayElement = document.getElementById("cpu-display");
		if (cpuDisplayElement) {
			cpuDisplayElement.textContent = "";
		}
	}
}

// Show window errors
window.addEventListener("unhandledrejection", (event) => {
	setStatus(event.reason, true);
});
window.onerror = (message, _filename, _lineno, _colno, _error) => {
	setStatus(message, true);
};

// Logging helpers
function logMessage(message) {
	const logElement = document.getElementById("log");
	const timestamp = new Date().toLocaleTimeString();
	logElement.innerHTML += `[${timestamp}] ${message}\n`;
	logElement.scrollTop = logElement.scrollHeight;
}
function setStatus(message, isError = false) {
	const statusElement = document.getElementById("status");
	statusElement.textContent = message;
	statusElement.style.backgroundColor = isError
		? "var(--status-error-bg)"
		: "var(--status-success-bg)";
	statusElement.style.color = isError
		? "var(--status-error-text)"
		: "var(--status-success-text)";
	logMessage(message);
}

// Player control buttons
document.getElementById("playButton").addEventListener("click", () => {
	backend.start();
	document.getElementById("playButton").disabled = true;
	document.getElementById("stopButton").disabled = false;
	document.getElementById("midiButton").disabled = false;
	document.getElementById("randomizeButton").disabled = false;
	document.getElementById("synthSelector").disabled = false;
	document.getElementById("voiceCountSelector").disabled = false;
	document.getElementById("metronomeCheckbox").disabled = false;
	document.getElementById("metronomeCheckbox").checked = true;
	document.getElementById("octaveInput").disabled = false;
	document.getElementById("velocityInput").disabled = false;
	effectManager.enableButtons();
	synthUI.init();
	modulationUI.init();
	setStatus("Player started");
	startCpuDisplay();
});

document.getElementById("stopButton").addEventListener("click", () => {
	// Disable MIDI if it's enabled
	if (midiEnabled) {
		disableMidi();
	}
	backend.stop();
	document.getElementById("playButton").disabled = false;
	document.getElementById("stopButton").disabled = true;
	document.getElementById("midiButton").disabled = true;
	document.getElementById("randomizeButton").disabled = true;
	document.getElementById("synthSelector").disabled = true;
	document.getElementById("voiceCountSelector").disabled = true;
	document.getElementById("metronomeCheckbox").disabled = true;
	document.getElementById("octaveInput").disabled = true;
	document.getElementById("velocityInput").disabled = true;
	effectManager.disableButtons();
	effectManager.removeAllEffects();
	synthUI.clear();
	modulationUI.clear();
	setStatus("Player stopped");
	stopCpuDisplay();
});

document.getElementById("metronomeCheckbox").addEventListener("change", (e) => {
	backend.setMetronomeEnabled(e.target.checked);
});

// MIDI support
let midiEnabled = false;
let midiAccess = null;
const currentMidiNotes = new Set();

const handleMidiMessage = (message) => {
	const data = message.data;
	const status = data[0] & 0xf0;
	const note = data[1];
	const velocity = data[2];
	if (status === 0x90 && velocity > 0) {
		// Note on
		if (!currentMidiNotes.has(note)) {
			currentMidiNotes.add(note);
			const normalizedVelocity = velocity / 127.0;
			backend.synthNoteOn(note, normalizedVelocity);
		}
	} else if (status === 0x80 || (status === 0x90 && velocity === 0)) {
		// Note off
		if (currentMidiNotes.has(note)) {
			currentMidiNotes.delete(note);
			backend.synthNoteOff(note);
		}
	}
};

const enableMidi = () => {
	if (!navigator.requestMIDIAccess) {
		return Promise.reject(new Error("Web MIDI API not supported"));
	}
	return navigator
		.requestMIDIAccess()
		.then((access) => {
			midiAccess = access;
			midiEnabled = true;
			const midiButton = document.getElementById("midiButton");
			midiButton.textContent = "Disable MIDI";
			midiButton.style.backgroundColor = "var(--button-active-bg)";
			// Start listening to MIDI input
			for (const input of midiAccess.inputs.values()) {
				input.onmidimessage = handleMidiMessage;
			}
			setStatus("MIDI input enabled. Play notes on your MIDI keyboard.");
		})
		.catch((err) => {
			setStatus("Failed to access MIDI: " + err.message, true);
			throw err;
		});
};

const disableMidi = () => {
	midiEnabled = false;
	const midiButton = document.getElementById("midiButton");
	midiButton.textContent = "Enable MIDI";
	midiButton.style.backgroundColor = "";
	// Stop listening to MIDI input
	if (midiAccess) {
		for (const input of midiAccess.inputs.values()) {
			input.onmidimessage = null;
		}
	}
	// Release all notes
	currentMidiNotes.forEach((note) => {
		backend.synthNoteOff(note);
	});
	currentMidiNotes.clear();
	setStatus("MIDI input disabled");
	return Promise.resolve();
};

document.getElementById("midiButton").addEventListener("click", () => {
	if (!midiEnabled) {
		enableMidi().catch((_err) => {
			// Error already logged in enableMidi
		});
	} else {
		disableMidi();
	}
});

// Stop all MIDI notes when leaving the page
document.addEventListener("visibilitychange", () => {
	if (document.visibilityState === "hidden" && midiEnabled) {
		currentMidiNotes.forEach((note) => {
			backend.synthNoteOff(note);
		});
		currentMidiNotes.clear();
	}
});

document.getElementById("randomizeButton").addEventListener("click", () => {
	const updates = backend.randomizeSynth();
	if (updates) {
		if (updates.parameters) {
			synthUI.applyUpdates(updates.parameters);
			modulationUI.applySourceParameterUpdates(updates.parameters);
		}
		if (updates.modulation) {
			modulationUI.applyModulationUpdates(updates.modulation);
		}
	}
});

document.getElementById("synthSelector").addEventListener("keydown", (e) => {
	e.preventDefault();
});
document.getElementById("synthSelector").addEventListener("change", (e) => {
	const synthType = parseInt(e.target.value, 10);
	backend.setActiveSynth(synthType);
	synthUI.init();
	modulationUI.init();
});

document
	.getElementById("voiceCountSelector")
	.addEventListener("keydown", (e) => {
		e.preventDefault();
	});
document
	.getElementById("voiceCountSelector")
	.addEventListener("change", (e) => {
		const voiceCount = parseInt(e.target.value, 10);
		backend.setSynthVoiceCount(voiceCount);
		synthUI.init();
		modulationUI.init();
		logMessage(`Changed voice count to ${voiceCount}`);
	});

// Octave controls
let currentOctave = 4;
const octaveInput = document.getElementById("octaveInput");

octaveInput.addEventListener("change", (e) => {
	const value = parseInt(e.target.value, 10);
	if (!Number.isNaN(value) && value >= 0 && value <= 8) {
		currentOctave = value;
	} else {
		octaveInput.value = currentOctave;
	}
});

// Velocity controls (1-127 MIDI range)
let currentVelocity = 80;
const velocityInput = document.getElementById("velocityInput");

velocityInput.addEventListener("change", (e) => {
	const value = parseInt(e.target.value, 10);
	if (!Number.isNaN(value) && value >= 1 && value <= 127) {
		currentVelocity = value;
	} else {
		velocityInput.value = currentVelocity;
	}
});

// Piano keyboard functionality
const pianoKeys = document.querySelectorAll(".piano-keys .key");
const activeKeyNotes = new Map();

const playNote = (keyIndex) => {
	// Stop any existing note for this key index to prevent hanging notes
	if (activeKeyNotes.has(keyIndex)) {
		const oldNote = activeKeyNotes.get(keyIndex);
		backend.synthNoteOff(oldNote);
	}

	const note = (currentOctave + 1) * 12 + parseInt(keyIndex, 10);
	activeKeyNotes.set(keyIndex, note);

	// Convert MIDI velocity (0-127) to normalized (0.0-1.0)
	const normalizedVelocity = currentVelocity / 127.0;
	backend.synthNoteOn(note, normalizedVelocity);
	const clickedKey = document.querySelector(`[data-key="${keyIndex}"]`);
	clickedKey?.classList.add("active");
};

const stopNote = (keyIndex) => {
	let note;
	if (activeKeyNotes.has(keyIndex)) {
		note = activeKeyNotes.get(keyIndex);
		activeKeyNotes.delete(keyIndex);
	} else {
		note = (currentOctave + 1) * 12 + parseInt(keyIndex, 10);
	}

	backend.synthNoteOff(note);
	const clickedKey = document.querySelector(`[data-key="${keyIndex}"]`);
	clickedKey?.classList.remove("active");
};

const allPianoKeys = new Map();
pianoKeys.forEach((key) => {
	const keyString = key.children[0].innerHTML;
	const keyValue = key.dataset.key;
	allPianoKeys.set(keyString, keyValue);
	key.addEventListener("mousedown", () => playNote(keyValue));
	key.addEventListener("click", () => stopNote(keyValue));
});
document.addEventListener("keydown", (e) => {
	if (!e.repeat && allPianoKeys.has(e.key)) {
		const keyValue = allPianoKeys.get(e.key);
		playNote(keyValue);
	}
});
document.addEventListener("keyup", (e) => {
	if (!e.repeat && allPianoKeys.has(e.key)) {
		const keyValue = allPianoKeys.get(e.key);
		stopNote(keyValue);
	}
});

// Synth UI Manager
class SynthUI {
	constructor() {
		this.container = document.getElementById("synth-parameters");
		this.controls = new Map();
	}

	init() {
		this.clear();
		const info = backend.getSynthParameters();
		if (info && info.parameters.length > 0) {
			this.container.classList.remove("hidden");
			info.parameters.forEach((param) => {
				const control = this.createParameterControl(param);
				this.container.appendChild(control);
			});
		}
	}

	clear() {
		this.container.innerHTML = "";
		if (!this.container.classList.contains("hidden")) {
			this.container.classList.add("hidden");
		}
		this.controls.clear();
	}

	createParameterControl(param) {
		const container = document.createElement("div");
		container.className = "parameter-control";

		const label = document.createElement("label");
		const nameSpan = document.createElement("span");
		nameSpan.textContent = param.name;
		const valueSpan = document.createElement("span");
		valueSpan.className = "param-value";
		valueSpan.style.cursor = "pointer";
		valueSpan.title = "Click to edit";

		label.appendChild(nameSpan);
		label.appendChild(valueSpan);

		const updateValueDisplay = (normalizedValue) => {
			const valueStr = backend.synthParameterValueToString(
				param.id,
				normalizedValue,
			);
			if (valueStr) {
				valueSpan.textContent = valueStr;
			}
		};

		let input;
		if (param.type === "Float" || param.type === "Integer") {
			input = document.createElement("input");
			input.tabIndex = -1;
			input.type = "range";
			input.min = 0.0;
			input.max = 1.0;
			input.step = param.step || 0.01;
			const normalized = param.default;
			input.value = normalized;
			updateValueDisplay(normalized);

			input.addEventListener("input", (e) => {
				const normalized = parseFloat(e.target.value);
				backend.setSynthParameterValue(param.id, normalized);
				updateValueDisplay(normalized);
			});

			// Add double-click handler to reset to default
			input.addEventListener("dblclick", () => {
				const defaultValue = param.default;
				input.value = defaultValue;
				backend.setSynthParameterValue(param.id, defaultValue);
				updateValueDisplay(defaultValue);
			});

			// Add click handler for value editing
			valueSpan.addEventListener("click", () => {
				this.showValueEditor(
					valueSpan,
					param.id,
					input,
					updateValueDisplay,
					false,
				);
			});
		} else if (param.type === "Boolean") {
			input = document.createElement("input");
			input.tabIndex = -1;
			input.type = "checkbox";
			input.checked = param.default > 0.5;
			updateValueDisplay(param.default);

			input.addEventListener("change", (e) => {
				const value = e.target.checked;
				const normalized = value ? 1.0 : 0.0;
				backend.setSynthParameterValue(param.id, normalized);
				updateValueDisplay(normalized);
			});
		} else if (param.type === "Enum") {
			input = document.createElement("select");
			input.tabIndex = -1;
			input.onkeydown = (e) => {
				e.preventDefault();
			};
			const default_index = Math.floor(
				param.default * (param.values.length - 1),
			);
			updateValueDisplay(param.default);
			param.values.forEach((val, idx) => {
				const option = document.createElement("option");
				option.tabIndex = -1;
				option.value = idx;
				option.textContent = val;
				if (idx === default_index) {
					option.selected = true;
				}
				input.appendChild(option);
			});

			input.addEventListener("change", (e) => {
				const idx = parseInt(e.target.value, 10);
				const normalized = idx / (param.values.length - 1);
				backend.setSynthParameterValue(param.id, normalized);
				updateValueDisplay(normalized);
			});
		}

		container.appendChild(label);
		if (input) {
			container.appendChild(input);
		}

		this.controls.set(param.id, { input, updateValueDisplay, param });
		return container;
	}

	showValueEditor(
		valueSpan,
		paramId,
		rangeInput,
		updateValueDisplay,
		isEffect,
		effectId = null,
	) {
		const currentText = valueSpan.textContent;
		const textInput = document.createElement("input");
		textInput.tabIndex = -1;
		textInput.type = "text";
		textInput.value = currentText;
		textInput.className = "param-value-editor";

		let isApplied = false;

		const applyValue = () => {
			if (isApplied) return;
			isApplied = true;

			const inputValue = textInput.value.trim();
			let normalizedValue;

			if (isEffect) {
				normalizedValue = backend.effectParameterStringToValue(
					effectId,
					paramId,
					inputValue,
				);
			} else {
				normalizedValue = backend.synthParameterStringToValue(
					paramId,
					inputValue,
				);
			}

			if (normalizedValue !== null) {
				if (isEffect) {
					backend.setEffectParameterValue(effectId, paramId, normalizedValue);
				} else {
					backend.setSynthParameterValue(paramId, normalizedValue);
				}
				rangeInput.value = normalizedValue;
				updateValueDisplay(normalizedValue);
			} else {
				// Invalid input, restore original value
				updateValueDisplay(parseFloat(rangeInput.value));
			}

			valueSpan.style.display = "";
			if (textInput.parentElement) {
				textInput.remove();
			}
		};

		const cancelEdit = () => {
			if (isApplied) return;
			isApplied = true;

			valueSpan.style.display = "";
			if (textInput.parentElement) {
				textInput.remove();
			}
		};

		textInput.addEventListener("keydown", (e) => {
			if (e.key === "Enter") {
				e.preventDefault();
				applyValue();
			} else if (e.key === "Escape") {
				e.preventDefault();
				cancelEdit();
			}
		});

		textInput.addEventListener("blur", () => {
			applyValue();
		});

		valueSpan.style.display = "none";
		valueSpan.parentElement.appendChild(textInput);
		textInput.focus();
		textInput.select();
	}

	applyUpdates(updates) {
		updates.forEach((update) => {
			const control = this.controls.get(update.id);
			if (control) {
				const { input, updateValueDisplay, param } = control;
				if (param.type === "Boolean") {
					input.checked = update.value > 0.5;
				} else if (param.type === "Enum") {
					const idx = Math.round(update.value * (param.values.length - 1));
					input.value = idx;
				} else {
					input.value = update.value;
				}
				updateValueDisplay(update.value);
			}
		});
	}
}

const synthUI = new SynthUI();

// Effect Manager Class
class EffectManager {
	constructor() {
		this.effects = new Map();
		this.chainElement = document.getElementById("effectsChain");
		this.availableEffects = [];
		this.initUI();
	}

	initUI() {
		// Get available effects from WASM and create buttons dynamically
		this.availableEffects = backend.getAvailableEffects();

		// Create add effect buttons dynamically
		const addEffectMenu = document.querySelector(".add-effect-menu");
		addEffectMenu.innerHTML = "";

		this.availableEffects.forEach((effectName) => {
			const button = document.createElement("button");
			button.id = `add${effectName}Btn`;
			button.tabIndex = -1;
			button.textContent = `+ ${effectName}`;
			button.disabled = true;
			button.addEventListener("click", () => this.addEffect(effectName));
			addEffectMenu.appendChild(button);
		});
	}

	enableButtons() {
		this.availableEffects.forEach((effectName) => {
			const button = document.getElementById(`add${effectName}Btn`);
			if (button) {
				button.disabled = false;
			}
		});
	}

	disableButtons() {
		this.availableEffects.forEach((effectName) => {
			const button = document.getElementById(`add${effectName}Btn`);
			if (button) {
				button.disabled = true;
			}
		});
	}

	addEffect(effectName) {
		const effectData = backend.addEffect(effectName);
		if (!effectData) {
			setStatus(`Failed to add ${effectName} effect`, true);
			return;
		}

		const effectId = effectData.effectId;
		const effectInfo = effectData.params;

		logMessage(`Added ${effectName} effect with ID ${effectId}`);
		this.addEffectCard(effectId, effectInfo);
	}

	addEffectCard(effectId, effectInfo) {
		// Remove empty message if present
		const emptyMessage = this.chainElement.querySelector(".empty-message");
		if (emptyMessage) {
			emptyMessage.remove();
		}

		const card = document.createElement("div");
		card.className = `effect-card ${effectInfo.name.toLowerCase()}`;
		card.dataset.effectId = effectId;

		const header = document.createElement("div");
		header.className = "effect-header";

		const nameSpan = document.createElement("span");
		nameSpan.className = "effect-name";
		nameSpan.textContent = effectInfo.name;

		const removeBtn = document.createElement("button");
		removeBtn.className = "remove-btn";
		removeBtn.tabIndex = -1;
		removeBtn.textContent = "×";
		removeBtn.addEventListener("click", () => this.removeEffect(effectId));

		header.appendChild(nameSpan);
		header.appendChild(removeBtn);

		const paramsDiv = document.createElement("div");
		paramsDiv.className = "effect-parameters";

		effectInfo.parameters.forEach((param) => {
			const control = this.createParameterControl(effectId, param);
			paramsDiv.appendChild(control);
		});

		card.appendChild(header);
		card.appendChild(paramsDiv);
		this.chainElement.appendChild(card);

		this.effects.set(effectId, {
			name: effectInfo.name,
			parameters: effectInfo.parameters,
		});
	}

	createParameterControl(effectId, param) {
		const container = document.createElement("div");
		container.className = "parameter-control";

		const label = document.createElement("label");
		const nameSpan = document.createElement("span");
		nameSpan.textContent = param.name;
		const valueSpan = document.createElement("span");
		valueSpan.className = "param-value";
		valueSpan.style.cursor = "pointer";
		valueSpan.title = "Click to edit";

		label.appendChild(nameSpan);
		label.appendChild(valueSpan);

		// Helper function to get and display parameter value string from WASM
		const updateValueDisplay = (normalizedValue) => {
			const valueStr = backend.effectParameterValueToString(
				effectId,
				param.id,
				normalizedValue,
			);
			if (valueStr) {
				valueSpan.textContent = valueStr;
			}
		};

		let input;
		if (param.type === "Float" || param.type === "Integer") {
			input = document.createElement("input");
			input.tabIndex = -1;
			input.type = "range";
			input.min = 0.0;
			input.max = 1.0;
			input.step = param.step || 0.01;
			const normalized = param.default;
			input.value = normalized;
			updateValueDisplay(normalized);

			input.addEventListener("input", (e) => {
				const normalized = parseFloat(e.target.value);
				backend.setEffectParameterValue(effectId, param.id, normalized);
				updateValueDisplay(normalized);
			});

			// Add double-click handler to reset to default
			input.addEventListener("dblclick", () => {
				const defaultValue = param.default;
				input.value = defaultValue;
				backend.setEffectParameterValue(effectId, param.id, defaultValue);
				updateValueDisplay(defaultValue);
			});

			// Add click handler for value editing
			valueSpan.addEventListener("click", () => {
				synthUI.showValueEditor(
					valueSpan,
					param.id,
					input,
					updateValueDisplay,
					true,
					effectId,
				);
			});
		} else if (param.type === "Boolean") {
			input = document.createElement("input");
			input.tabIndex = -1;
			input.type = "checkbox";
			input.checked = param.default;
			updateValueDisplay(param.default ? 1.0 : 0.0);

			input.addEventListener("change", (e) => {
				const value = e.target.checked;
				const normalized = value ? 1.0 : 0.0;
				backend.setEffectParameterValue(effectId, param.id, normalized);
				updateValueDisplay(normalized);
			});
		} else if (param.type === "Enum") {
			input = document.createElement("select");
			input.tabIndex = -1;
			input.onkeydown = (e) => {
				e.preventDefault();
			};
			const default_index = Math.floor(
				param.default * (param.values.length - 1),
			);
			updateValueDisplay(param.default);
			param.values.forEach((val, idx) => {
				const option = document.createElement("option");
				option.tabIndex = -1;
				option.value = idx;
				option.textContent = val;
				if (idx === default_index) {
					option.selected = true;
				}
				input.appendChild(option);
			});

			input.addEventListener("change", (e) => {
				const idx = parseInt(e.target.value, 10);
				const normalized = idx / (param.values.length - 1);
				backend.setEffectParameterValue(effectId, param.id, normalized);
				updateValueDisplay(normalized);
			});
		}

		container.appendChild(label);
		if (input) {
			container.appendChild(input);
		}

		return container;
	}

	removeEffect(effectId) {
		const result = backend.removeEffect(effectId);
		if (result === 0) {
			const card = this.chainElement.querySelector(
				`[data-effect-id="${effectId}"]`,
			);
			if (card) {
				card.remove();
			}
			this.effects.delete(effectId);
			logMessage(`Removed effect ${effectId}`);

			// Show empty message if no effects remain
			if (this.effects.size === 0) {
				const emptyMessage = document.createElement("div");
				emptyMessage.className = "empty-message";
				emptyMessage.textContent =
					"No effects added yet. Add an effect using the buttons above.";
				this.chainElement.appendChild(emptyMessage);
			}
		} else {
			setStatus(`Failed to remove effect ${effectId}`, true);
		}
	}

	removeAllEffects() {
		if (this.effects.size === 0) {
			// no effects present
			return;
		}

		// Get all effect IDs before removing them
		const effectIds = Array.from(this.effects.keys());

		// Remove each effect
		effectIds.forEach((effectId) => {
			const result = backend.removeEffect(effectId);
			if (result === 0) {
				logMessage(`Removed effect ${effectId}`);
			} else {
				logMessage(`Failed to remove effect ${effectId}`);
			}
		});

		// Clear all effect cards from the UI
		const cards = this.chainElement.querySelectorAll(".effect-card");
		cards.forEach((card) => {
			card.remove();
		});

		// Clear the effects map
		this.effects.clear();

		// Show empty message
		const emptyMessage = document.createElement("div");
		emptyMessage.className = "empty-message";
		emptyMessage.textContent =
			"No effects added yet. Add an effect using the buttons above.";
		this.chainElement.appendChild(emptyMessage);
	}
}

// Modulation Matrix UI Class
class ModulationMatrixUI {
	constructor() {
		this.sources = [];
		this.targets = [];
		this.connections = new Map(); // key: "sourceId-targetId", value: {sourceId, targetId, amount, bipolar}
		this.sourceParamControls = new Map(); // key: paramId, value: {input, updateValueDisplay, param}
		this.containerElement = document.getElementById("modulationSection");
		this.sourcesElement = document.getElementById("modulationSources");
	}

	init() {
		this.clear();
		const sources = backend.getModulationSources();
		const targets = backend.getModulationTargets();

		if (!sources || !targets || sources.length === 0) {
			// No modulation support for this synth
			this.containerElement.classList.add("hidden");
			return;
		}

		this.sources = sources;
		this.targets = targets;
		this.containerElement.classList.remove("hidden");

		// Create source cards
		sources.forEach((source) => {
			const card = this.createSourceCard(source);
			this.sourcesElement.appendChild(card);
			this.updateAvailableTargets(source.id);
		});
	}

	clear() {
		this.sourcesElement.innerHTML = "";
		this.connections.clear();
		this.sourceParamControls.clear();
		this.sources = [];
		this.targets = [];
		this.containerElement.classList.add("hidden");
	}

	createSourceCard(source) {
		const card = document.createElement("div");
		card.className = "mod-source-card";
		card.dataset.sourceId = source.id;

		// Header
		const header = document.createElement("div");
		header.className = "mod-source-header";

		const nameSpan = document.createElement("span");
		nameSpan.className = "mod-source-name";
		nameSpan.textContent = source.name;

		const polarityBadge = document.createElement("span");
		polarityBadge.className = "mod-polarity-badge";
		polarityBadge.textContent = source.polarity === "bipolar" ? "±" : "+";

		header.appendChild(nameSpan);
		header.appendChild(polarityBadge);

		// Config parameters
		const paramsDiv = document.createElement("div");
		paramsDiv.className = "mod-source-params";

		if (source.parameters && source.parameters.length > 0) {
			source.parameters.forEach((param) => {
				const control = this.createParameterControl(param);
				paramsDiv.appendChild(control);
			});
		}

		// Add target dropdown
		const addTargetDiv = document.createElement("div");
		addTargetDiv.className = "mod-add-target";

		const addTargetSelect = document.createElement("select");
		addTargetSelect.tabIndex = -1;
		addTargetSelect.onkeydown = (e) => {
			e.preventDefault();
		};

		const placeholderOption = document.createElement("option");
		placeholderOption.value = "";
		placeholderOption.textContent = "+ Target";
		placeholderOption.disabled = true;
		placeholderOption.selected = true;
		addTargetSelect.appendChild(placeholderOption);

		addTargetSelect.addEventListener("change", (e) => {
			const targetId = parseInt(e.target.value, 10);
			if (targetId) {
				this.addConnection(source.id, targetId, source.polarity === "bipolar");
				addTargetSelect.value = "";
			}
		});

		addTargetDiv.appendChild(addTargetSelect);

		// Connections container
		const connectionsDiv = document.createElement("div");
		connectionsDiv.className = "mod-connections";
		connectionsDiv.dataset.sourceId = source.id;

		card.appendChild(header);
		if (source.parameters && source.parameters.length > 0) {
			card.appendChild(paramsDiv);
		}
		card.appendChild(addTargetDiv);
		card.appendChild(connectionsDiv);

		return card;
	}

	createParameterControl(param) {
		const container = document.createElement("div");
		container.className = "parameter-control";

		const label = document.createElement("label");
		const nameSpan = document.createElement("span");
		nameSpan.textContent = param.name;
		const valueSpan = document.createElement("span");
		valueSpan.className = "param-value";
		valueSpan.style.cursor = "pointer";
		valueSpan.title = "Click to edit";

		label.appendChild(nameSpan);
		label.appendChild(valueSpan);

		const updateValueDisplay = (normalizedValue) => {
			const valueStr = backend.synthParameterValueToString(
				param.id,
				normalizedValue,
			);
			if (valueStr) {
				valueSpan.textContent = valueStr;
			}
		};

		let input;
		if (param.type === "Float" || param.type === "Integer") {
			input = document.createElement("input");
			input.tabIndex = -1;
			input.type = "range";
			input.min = 0.0;
			input.max = 1.0;
			input.step = param.step || 0.01;
			const normalized = param.default;
			input.value = normalized;
			updateValueDisplay(normalized);

			input.addEventListener("input", (e) => {
				const normalized = parseFloat(e.target.value);
				backend.setSynthParameterValue(param.id, normalized);
				updateValueDisplay(normalized);
			});

			// Add double-click handler to reset to default
			input.addEventListener("dblclick", () => {
				const defaultValue = param.default;
				input.value = defaultValue;
				backend.setSynthParameterValue(param.id, defaultValue);
				updateValueDisplay(defaultValue);
			});

			// Add click handler for value editing
			valueSpan.addEventListener("click", () => {
				synthUI.showValueEditor(
					valueSpan,
					param.id,
					input,
					updateValueDisplay,
					false,
				);
			});
		} else if (param.type === "Boolean") {
			input = document.createElement("input");
			input.tabIndex = -1;
			input.type = "checkbox";
			input.checked = param.default > 0.5;
			updateValueDisplay(param.default);

			input.addEventListener("change", (e) => {
				const value = e.target.checked;
				const normalized = value ? 1.0 : 0.0;
				backend.setSynthParameterValue(param.id, normalized);
				updateValueDisplay(normalized);
			});
		} else if (param.type === "Enum") {
			input = document.createElement("select");
			input.tabIndex = -1;
			input.onkeydown = (e) => {
				e.preventDefault();
			};
			const default_index = Math.floor(
				param.default * (param.values.length - 1),
			);
			updateValueDisplay(param.default);
			param.values.forEach((val, idx) => {
				const option = document.createElement("option");
				option.onkeydown = (e) => {
					e.preventDefault();
				};
				option.tabIndex = -1;
				option.value = idx;
				option.textContent = val;
				if (idx === default_index) {
					option.selected = true;
				}
				input.appendChild(option);
			});

			input.addEventListener("change", (e) => {
				const idx = parseInt(e.target.value, 10);
				const normalized = idx / (param.values.length - 1);
				backend.setSynthParameterValue(param.id, normalized);
				updateValueDisplay(normalized);
			});
		}

		container.appendChild(label);
		if (input) {
			container.appendChild(input);
		}

		this.sourceParamControls.set(param.id, {
			input,
			updateValueDisplay,
			param,
		});
		return container;
	}

	updateAvailableTargets(sourceId) {
		const card = this.sourcesElement.querySelector(
			`.mod-source-card[data-source-id="${sourceId}"]`,
		);
		if (!card) return;

		const select = card.querySelector(".mod-add-target select");
		if (!select) return;

		// Clear existing options except placeholder
		while (select.options.length > 1) {
			select.remove(1);
		}

		// Get connected targets for this source
		const connectedTargets = Array.from(this.connections.values())
			.filter((conn) => conn.sourceId === sourceId)
			.map((conn) => conn.targetId);

		// Add available targets
		this.targets.forEach((target) => {
			if (!connectedTargets.includes(target.id)) {
				const option = document.createElement("option");
				option.value = target.id;
				option.textContent = target.name;
				option.tabIndex = -1;
				option.onkeydown = (e) => {
					e.preventDefault();
				};
				select.appendChild(option);
			}
		});
	}

	addConnection(sourceId, targetId, defaultBipolar) {
		const key = `${sourceId}-${targetId}`;
		if (this.connections.has(key)) return;

		const amount = 0.5;
		const bipolar = defaultBipolar;

		// Call backend
		const result = backend.setModulation(sourceId, targetId, amount, bipolar);
		if (result !== 0) {
			setStatus(`Failed to add modulation routing`, true);
			return;
		}

		// Store connection
		this.connections.set(key, { sourceId, targetId, amount, bipolar });

		// Create UI element
		const connectionsDiv = this.sourcesElement.querySelector(
			`.mod-connections[data-source-id="${sourceId}"]`,
		);
		if (connectionsDiv) {
			const connectionElement = this.createConnectionElement(
				sourceId,
				targetId,
				amount,
				bipolar,
			);
			connectionsDiv.appendChild(connectionElement);
		}

		// Update dropdown
		this.updateAvailableTargets(sourceId);
	}

	createConnectionElement(sourceId, targetId, amount, bipolar) {
		const container = document.createElement("div");
		container.className = "mod-connection";
		container.dataset.connectionKey = `${sourceId}-${targetId}`;

		const target = this.targets.find((t) => t.id === targetId);
		const targetName = target ? target.name : targetId.toString();

		// Header with name and remove button
		const header = document.createElement("div");
		header.className = "mod-connection-header";

		const nameSpan = document.createElement("span");
		nameSpan.className = "mod-connection-name";
		nameSpan.textContent = targetName;
		nameSpan.title = targetName;

		const removeBtn = document.createElement("button");
		removeBtn.className = "mod-connection-remove";
		removeBtn.textContent = "×";
		removeBtn.tabIndex = -1;
		removeBtn.addEventListener("click", () => {
			this.removeConnection(sourceId, targetId);
		});

		header.appendChild(nameSpan);
		header.appendChild(removeBtn);

		// Slider controls
		const controlsDiv = document.createElement("div");
		controlsDiv.className = "mod-connection-controls";

		const slider = document.createElement("input");
		slider.type = "range";
		slider.min = -1;
		slider.max = 1;
		slider.step = 0.01;
		slider.value = amount;
		slider.tabIndex = -1;

		const valueSpan = document.createElement("span");
		valueSpan.className = "mod-connection-value";
		valueSpan.textContent = amount.toFixed(2);

		slider.addEventListener("input", (e) => {
			const newAmount = parseFloat(e.target.value);
			valueSpan.textContent = newAmount.toFixed(2);
			this.setConnectionAmount(sourceId, targetId, newAmount, bipolar);
		});

		controlsDiv.appendChild(slider);
		controlsDiv.appendChild(valueSpan);

		container.appendChild(header);
		container.appendChild(controlsDiv);

		return container;
	}

	setConnectionAmount(sourceId, targetId, amount, bipolar) {
		const key = `${sourceId}-${targetId}`;
		const result = backend.setModulation(sourceId, targetId, amount, bipolar);
		if (result === 0) {
			const conn = this.connections.get(key);
			if (conn) {
				conn.amount = amount;
			}
		}
	}

	removeConnection(sourceId, targetId) {
		const key = `${sourceId}-${targetId}`;
		const result = backend.clearModulation(sourceId, targetId);
		if (result === 0) {
			this.connections.delete(key);

			// Remove UI element
			const connectionElement = this.sourcesElement.querySelector(
				`.mod-connection[data-connection-key="${key}"]`,
			);
			if (connectionElement) {
				connectionElement.remove();
			}

			// Update dropdown
			this.updateAvailableTargets(sourceId);
		} else {
			setStatus(`Failed to remove modulation routing`, true);
		}
	}

	applyModulationUpdates(updates) {
		// Clear all existing connections
		Array.from(this.connections.keys()).forEach((key) => {
			const [sourceId, targetId] = key.split("-").map((id) => parseInt(id, 10));
			backend.clearModulation(sourceId, targetId);
		});
		this.connections.clear();

		// Clear UI
		this.sourcesElement.querySelectorAll(".mod-connections").forEach((div) => {
			div.innerHTML = "";
		});

		// Apply new routings
		updates.forEach((update) => {
			this.addConnection(update.source_id, update.target_id, update.bipolar);
			// Update amount
			const key = `${update.source_id}-${update.target_id}`;
			const conn = this.connections.get(key);
			if (conn) {
				conn.amount = update.amount;
				// Update UI slider
				const slider = this.sourcesElement.querySelector(
					`.mod-connection[data-connection-key="${key}"] input[type="range"]`,
				);
				if (slider) {
					slider.value = update.amount;
					const valueSpan = slider.nextElementSibling;
					if (valueSpan) {
						valueSpan.textContent = update.amount.toFixed(2);
					}
				}
			}
		});

		// Update all dropdowns
		this.sources.forEach((source) => {
			this.updateAvailableTargets(source.id);
		});
	}

	applySourceParameterUpdates(updates) {
		updates.forEach((update) => {
			const control = this.sourceParamControls.get(update.id);
			if (control) {
				const { input, updateValueDisplay, param } = control;
				if (param.type === "Boolean") {
					input.checked = update.value > 0.5;
				} else if (param.type === "Enum") {
					const idx = Math.round(update.value * (param.values.length - 1));
					input.value = idx;
				} else {
					input.value = update.value;
				}
				updateValueDisplay(update.value);
			}
		});
	}
}

const modulationUI = new ModulationMatrixUI();

// Initialize effect manager when WASM is ready
let effectManager;

// Emscripten Module setup
Module = {
	onRuntimeInitialized: () => {
		effectManager = new EffectManager();
		document.getElementById("playButton").disabled = false;
		setStatus("WASM module loaded and ready");
	},
	print: (...args) => logMessage(`[stdout]: ${args.join(" ")}`),
	printErr: (...args) => logMessage(`[stderr]: ${args.join(" ")}`),
};
