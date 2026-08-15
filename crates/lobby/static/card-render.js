/* ─────────────────────────────────────────────────────────────
 * card-render.js  —  zero-asset playing-card renderer
 *
 * Public API (attached to window.cardRender):
 *   SUITS_GLYPH    ['♠','♥','♦','♣']            // index = suit (s)
 *   SUIT_COLOR     ['black','red','red','black']
 *   RANK_TEXT      ['A','2',…,'10','J','Q','K'] // index = rank (r)
 *   cardFromCodes(s, r)                          -> {s, r}
 *   isCard(c)                                    -> boolean
 *   renderCardEl(c, opts?)                       -> HTMLDivElement
 *   renderCardInline(c)                          -> string ("♥A")
 *   renderHandEl(cards, opts?)                   -> HTMLDivElement
 *   renderOpponentsEl(opponents, opts?)          -> HTMLDivElement
 *   renderTableEl(slots, opts?)                  -> HTMLDivElement
 *
 * Wire format (matches what the TYP game crate will emit):
 *   card = { s: 0..3, r: 0..12 }   // s=Spade Heart Diamond Club; r=A 2..10 J Q K
 *   null / undefined                // empty slot, face-down card
 *
 * No external dependencies, no images, no fonts (beyond system Unicode
 * support for suit glyphs ♠♥♦♣ which is universal since Unicode 1.1).
 * ───────────────────────────────────────────────────────────── */
(function () {
    "use strict";

    var SUITS_GLYPH = ['\u2660', '\u2665', '\u2666', '\u2663']; // ♠ ♥ ♦ ♣
    var SUIT_COLOR  = ['black', 'red', 'red', 'black'];
    var SUIT_NAME   = ['spade', 'heart', 'diamond', 'club'];
    var RANK_TEXT   = ['A', '2', '3', '4', '5', '6', '7', '8', '9', '10', 'J', 'Q', 'K'];

    function cardFromCodes(s, r) {
        return { s: s | 0, r: r | 0 };
    }

    function isCard(c) {
        return c && typeof c === "object"
            && Number.isInteger(c.s) && c.s >= 0 && c.s < 4
            && Number.isInteger(c.r) && c.r >= 0 && c.r < 13;
    }

    /* Build a card <div>. c may be null/undefined for a face-down/empty slot. */
    function renderCardEl(c, opts) {
        opts = opts || {};
        var el = document.createElement("div");
        if (!isCard(c)) {
            // Empty slot (no card there yet) vs face-down (card present but hidden).
            if (opts.faceDown) {
                el.className = "play-card back";
                el.setAttribute("aria-label", "face-down card");
            } else {
                el.className = "play-card empty";
                el.setAttribute("aria-label", "empty slot");
            }
            return el;
        }
        var color = SUIT_COLOR[c.s];
        var glyph = SUITS_GLYPH[c.s];
        var rank  = RANK_TEXT[c.r];
        el.className = "play-card " + color;
        if (opts.selected)   el.classList.add("selected");
        if (opts.clickable)  el.classList.add("clickable");
        if (opts.disabled)   el.classList.add("disabled");
        el.setAttribute("data-s", String(c.s));
        el.setAttribute("data-r", String(c.r));
        el.setAttribute("aria-label", rank + " of " + SUIT_NAME[c.s]);
        if (opts.cardId !== undefined) el.setAttribute("data-card-id", String(opts.cardId));
        // Top-left corner
        var top = document.createElement("div");
        top.className = "corner";
        var topR = document.createElement("span"); topR.className = "rank"; topR.textContent = rank;
        var topS = document.createElement("span"); topS.className = "suit"; topS.textContent = glyph;
        top.appendChild(topR); top.appendChild(topS);
        el.appendChild(top);
        // Center pip
        var pip = document.createElement("div");
        pip.className = "pip";
        pip.textContent = glyph;
        el.appendChild(pip);
        // Bottom-right corner (mirrored)
        var bot = document.createElement("div");
        bot.className = "corner bot";
        var botR = document.createElement("span"); botR.className = "rank"; botR.textContent = rank;
        var botS = document.createElement("span"); botS.className = "suit"; botS.textContent = glyph;
        bot.appendChild(botR); bot.appendChild(botS);
        el.appendChild(bot);
        // Click handler
        if (typeof opts.onClick === "function") {
            el.addEventListener("click", function (ev) {
                if (el.classList.contains("disabled")) return;
                opts.onClick(c, opts.cardId, ev);
            });
        }
        return el;
    }

    /* Compact inline form: "♥A", "♠K", "♦10" — for logs / opponent badges. */
    function renderCardInline(c) {
        if (!isCard(c)) return "?";
        return SUITS_GLYPH[c.s] + RANK_TEXT[c.r];
    }
    function renderCardInlineEl(c) {
        var span = document.createElement("span");
        if (!isCard(c)) {
            span.className = "play-card-inline";
            span.textContent = "?";
            return span;
        }
        span.className = "play-card-inline " + SUIT_COLOR[c.s];
        span.textContent = renderCardInline(c);
        return span;
    }

    /* Own hand (face-up, fan layout). cards = array of card objects. */
    function renderHandEl(cards, opts) {
        opts = opts || {};
        var wrap = document.createElement("div");
        wrap.className = "hand" + (opts.className ? " " + opts.className : "");
        if (opts.ariaLabel) wrap.setAttribute("aria-label", opts.ariaLabel);
        (cards || []).forEach(function (c, i) {
            var cardOpts = {
                clickable: !!opts.onCardClick,
                selected: !!opts.selectedSet && opts.selectedSet.has(i),
                disabled: !!opts.disabledSet && opts.disabledSet.has(i),
                cardId: i,
                faceDown: !!opts.faceDown,
            };
            if (opts.onCardClick) {
                cardOpts.onClick = function (card, idx) {
                    opts.onCardClick(card, idx);
                };
            }
            wrap.appendChild(renderCardEl(c, cardOpts));
        });
        return wrap;
    }

    /* Opponent row.
     * opponents = [{ uid, name, score, prediction, hasPredicted, isActive, isMe, cards? }]
     *   cards: array of card objects, or null/undefined to render all face-down
     *   cardsCount: number — alternative to cards (renders N back-cards)
     */
    function renderOpponentsEl(opponents, opts) {
        opts = opts || {};
        var wrap = document.createElement("div");
        wrap.className = "opponents";
        (opponents || []).forEach(function (o) {
            var cell = document.createElement("div");
            cell.className = "opponent";

            // Top: name + active dot
            var name = document.createElement("div");
            name.className = "seat-name";
            var dot = document.createElement("span");
            dot.className = "turn-dot" + (o.isActive ? "" : " idle");
            name.appendChild(dot);
            var label = document.createElement("span");
            label.textContent = o.name + (o.isMe ? "（你）" : "");
            name.appendChild(label);
            cell.appendChild(name);

            // Middle: hand (always face-down for opponents by default)
            var count = o.cardsCount != null
                ? o.cardsCount
                : (Array.isArray(o.cards) ? o.cards.length : 5);
            var handCards = (opts.faceUp === true && Array.isArray(o.cards))
                ? o.cards
                : null; // null array => all back cards
            var handOpts = {
                faceDown: opts.faceUp !== true,
                ariaLabel: (o.name || "opponent") + "'s hand",
            };
            var hand = renderHandEl(handCards, handOpts);
            // If we know the count but not the cards, append N back cards manually.
            if (!Array.isArray(o.cards) && count > 0) {
                for (var i = 0; i < count; i++) hand.appendChild(renderCardEl(null, { faceDown: true }));
            }
            cell.appendChild(hand);

            // Bottom: prediction + score
            var pred = document.createElement("div");
            pred.className = "prediction" + (o.hasPredicted ? " done" : "");
            if (o.prediction != null) {
                pred.textContent = "预测 #" + o.prediction;
            } else if (o.hasPredicted === false) {
                pred.textContent = "未预测";
            } else {
                pred.textContent = "";
            }
            cell.appendChild(pred);

            var score = document.createElement("div");
            score.className = "score";
            score.textContent = (o.score != null ? o.score : 0) + " 分";
            cell.appendChild(score);

            wrap.appendChild(cell);
        });
        return wrap;
    }

    /* Center table (5 slots in a ring).
     * slots = [{ uid, name, card | null }]
     */
    function renderTableEl(slots, opts) {
        opts = opts || {};
        var wrap = document.createElement("div");
        wrap.className = "play-table";
        wrap.setAttribute("aria-label", "play area");
        (slots || []).slice(0, 5).forEach(function (s, i) {
            var slot = document.createElement("div");
            slot.className = "slot";
            slot.setAttribute("data-pos", String(i));
            var label = document.createElement("div");
            label.className = "seat-label";
            label.textContent = s.name || ("seat " + i);
            slot.appendChild(label);
            slot.appendChild(renderCardEl(s.card || null, {
                faceDown: !!s.faceDown,
            }));
            wrap.appendChild(slot);
        });
        return wrap;
    }

    window.cardRender = {
        SUITS_GLYPH: SUITS_GLYPH,
        SUIT_COLOR:  SUIT_COLOR,
        SUIT_NAME:   SUIT_NAME,
        RANK_TEXT:   RANK_TEXT,
        cardFromCodes: cardFromCodes,
        isCard: isCard,
        renderCardEl: renderCardEl,
        renderCardInline: renderCardInline,
        renderCardInlineEl: renderCardInlineEl,
        renderHandEl: renderHandEl,
        renderOpponentsEl: renderOpponentsEl,
        renderTableEl: renderTableEl,
    };
})();
