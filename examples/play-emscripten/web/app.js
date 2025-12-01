// Backend wrapper functions
const backend = {
  start() {
    ccall("start", null, [], []);
  },
  stop() {
    ccall("stop", null, [], []);
  },
  synthNoteOn(key) {
    ccall("synth_note_on", null, ["number"], [key]);
  },
  synthNoteOff(key) {
    ccall("synth_note_off", null, ["number"], [key]);
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
  getEffectParameterString(effectId, paramId, normalizedValue) {
    const valuePtr = ccall(
      "get_effect_parameter_string",
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
  setEffectParameterValue(effectId, paramId, normalizedValue) {
    ccall(
      "set_effect_parameter_value",
      null,
      ["number", "number", "number"],
      [effectId, paramId, normalizedValue],
    );
  },
};

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
  statusElement.style.backgroundColor = isError ? "#ffebee" : "#e8f5e9";
  statusElement.style.color = isError ? "#c62828" : "#2e7d32";
  logMessage(message);
}

// Player control buttons
document.getElementById("playButton").addEventListener("click", () => {
  backend.start();
  document.getElementById("playButton").disabled = true;
  document.getElementById("stopButton").disabled = false;
  effectManager.enableButtons();
  setStatus("Player started");
});

document.getElementById("stopButton").addEventListener("click", () => {
  backend.stop();
  document.getElementById("playButton").disabled = false;
  document.getElementById("stopButton").disabled = true;
  effectManager.disableButtons();
  effectManager.removeAllEffects();
  setStatus("Player stopped");
});

// Piano keyboard functionality
const pianoKeys = document.querySelectorAll(".piano-keys .key");
const playNote = (key) => {
  backend.synthNoteOn(key);
  const clickedKey = document.querySelector(`[data-key="${key}"]`);
  clickedKey?.classList.add("active");
};
const stopNote = (key) => {
  backend.synthNoteOff(key);
  const clickedKey = document.querySelector(`[data-key="${key}"]`);
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

    this.effects.set(effectId, { name: effectInfo.name, parameters: effectInfo.parameters });
  }

  createParameterControl(effectId, param) {
    const container = document.createElement("div");
    container.className = "parameter-control";

    const label = document.createElement("label");
    const nameSpan = document.createElement("span");
    nameSpan.textContent = param.name;
    const valueSpan = document.createElement("span");
    valueSpan.className = "param-value";

    label.appendChild(nameSpan);
    label.appendChild(valueSpan);

    // Helper function to get and display parameter value string from WASM
    const updateValueDisplay = (normalizedValue) => {
      const valueStr = backend.getEffectParameterString(effectId, param.id, normalizedValue);
      if (valueStr) {
        valueSpan.textContent = valueStr;
      }
    };

    let input;
    if (param.type === "Float") {
      input = document.createElement("input");
      input.type = "range";
      input.min = "0";
      input.max = "1";
      input.step = "0.01";
      const normalized = param.default;
      input.value = normalized;
      updateValueDisplay(normalized);

      input.addEventListener("input", (e) => {
        const normalized = parseFloat(e.target.value);
        backend.setEffectParameterValue(effectId, param.id, normalized);
        updateValueDisplay(normalized);
      });
    } else if (param.type === "Integer") {
      input = document.createElement("input");
      input.type = "range";
      input.min = "0";
      input.max = "1";
      input.step = "0.01";
      const normalized = param.default;
      input.value = normalized;
      updateValueDisplay(normalized);

      input.addEventListener("input", (e) => {
        const normalizedValue = parseFloat(e.target.value);
        backend.setEffectParameterValue(effectId, param.id, normalizedValue);
        updateValueDisplay(normalizedValue);
      });
    } else if (param.type === "Boolean") {
      input = document.createElement("input");
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
      const default_index = Math.floor(param.default * (param.values.length - 1));
      updateValueDisplay(param.default);
      param.values.forEach((val, idx) => {
        const option = document.createElement("option");
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
      const card = this.chainElement.querySelector(`[data-effect-id="${effectId}"]`);
      if (card) {
        card.remove();
      }
      this.effects.delete(effectId);
      logMessage(`Removed effect ${effectId}`);

      // Show empty message if no effects remain
      if (this.effects.size === 0) {
        const emptyMessage = document.createElement("div");
        emptyMessage.className = "empty-message";
        emptyMessage.textContent = "No effects added yet. Add an effect using the buttons above.";
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
    emptyMessage.textContent = "No effects added yet. Add an effect using the buttons above.";
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
