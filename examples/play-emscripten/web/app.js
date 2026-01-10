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
	synthNoteOn(key) {
		ccall("synth_note_on", null, ["number"], [key]);
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
	document.getElementById("randomizeButton").disabled = false;
	document.getElementById("synthSelector").disabled = false;
	document.getElementById("voiceCountSelector").disabled = false;
	document.getElementById("metronomeCheckbox").disabled = false;
	document.getElementById("metronomeCheckbox").checked = true;
	document.getElementById("octaveDown").disabled = false;
	document.getElementById("octaveUp").disabled = false;
	effectManager.enableButtons();
	synthUI.init();
	setStatus("Player started");
	startCpuDisplay();
});

document.getElementById("stopButton").addEventListener("click", () => {
	backend.stop();
	document.getElementById("playButton").disabled = false;
	document.getElementById("stopButton").disabled = true;
	document.getElementById("randomizeButton").disabled = true;
	document.getElementById("synthSelector").disabled = true;
	document.getElementById("voiceCountSelector").disabled = true;
	document.getElementById("metronomeCheckbox").disabled = true;
	document.getElementById("octaveDown").disabled = true;
	document.getElementById("octaveUp").disabled = true;
	effectManager.disableButtons();
	effectManager.removeAllEffects();
	synthUI.clear();
	setStatus("Player stopped");
	stopCpuDisplay();
});

document.getElementById("metronomeCheckbox").addEventListener("change", (e) => {
	backend.setMetronomeEnabled(e.target.checked);
});

document.getElementById("randomizeButton").addEventListener("click", () => {
	const updates = backend.randomizeSynth();
	if (updates) {
		synthUI.applyUpdates(updates);
	}
});

document.getElementById("synthSelector").addEventListener("change", (e) => {
	const synthType = parseInt(e.target.value, 10);
	backend.setActiveSynth(synthType);
	synthUI.init();
});

document
	.getElementById("voiceCountSelector")
	.addEventListener("change", (e) => {
		const voiceCount = parseInt(e.target.value, 10);
		backend.setSynthVoiceCount(voiceCount);
		synthUI.init();
		logMessage(`Changed voice count to ${voiceCount}`);
	});

// Octave controls
let currentOctave = 4;

function updateOctaveDisplay() {
	document.getElementById("octaveDisplay").textContent =
		`Octave: ${currentOctave}`;
}

document.getElementById("octaveDown").addEventListener("click", () => {
	if (currentOctave > 0) {
		currentOctave--;
		updateOctaveDisplay();
	}
});

document.getElementById("octaveUp").addEventListener("click", () => {
	if (currentOctave < 8) {
		currentOctave++;
		updateOctaveDisplay();
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

	const note = (currentOctave + 1) * 12 + parseInt(keyIndex);
	activeKeyNotes.set(keyIndex, note);

	backend.synthNoteOn(note);
	const clickedKey = document.querySelector(`[data-key="${keyIndex}"]`);
	clickedKey?.classList.add("active");
};

const stopNote = (keyIndex) => {
	let note;
	if (activeKeyNotes.has(keyIndex)) {
		note = activeKeyNotes.get(keyIndex);
		activeKeyNotes.delete(keyIndex);
	} else {
		note = (currentOctave + 1) * 12 + parseInt(keyIndex);
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
