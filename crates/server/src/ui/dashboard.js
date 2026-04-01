// Skrills Dashboard Client-Side Logic
(function() {
    let skills = [];
    let events = [];
    let mcpServers = [];
    let selectedSkill = null;
    let skillsTotal = 0;
    let loadingMore = false;
    const PAGE_SIZE = 50;

    let initialized = false;
    let sortOrder = 'discovery'; // 'discovery' | 'alpha'
    let skillInvocationCounts = {}; // skill_name -> total_invocations

    async function refresh() {
        try {
            // On first load, fetch page 1. On subsequent refreshes, only
            // update the total count — don't nuke already-loaded skills.
            const res = await fetch('/api/skills?limit=' + PAGE_SIZE);
            const data = await res.json();
            skillsTotal = data.total || 0;
            document.getElementById('skill-count').textContent = skillsTotal;

            if (!initialized) {
                skills = (data.items || []).map((s, i) => ({ ...s, _idx: i }));
                if (sortOrder === 'alpha') {
                    skills.sort((a, b) => a.name.localeCompare(b.name));
                }
                renderSkills();
                initialized = true;
            }

            // Clear selected skill if it no longer exists in the refreshed list
            if (selectedSkill && !skills.find(s => s.name === selectedSkill.name)) {
                selectedSkill = null;
                renderMetrics();
            }
        } catch (e) {
            console.error('Failed to fetch skills:', e);
        }

        try {
            const res = await fetch('/api/metrics/events');
            const data = await res.json();
            events = data.events || [];
            document.getElementById('event-count').textContent = events.length;
            renderEvents();
        } catch (e) {
            console.error('Failed to fetch events:', e);
        }

        try {
            const res = await fetch('/api/metrics/analytics');
            const data = await res.json();
            document.getElementById('invocation-count').textContent = data.total_invocations || 0;
        } catch (e) {
            console.error('Failed to fetch analytics:', e);
        }

        try {
            const res = await fetch('/api/metrics/analytics/top');
            const data = await res.json();
            skillInvocationCounts = {};
            (data.skills || []).forEach(s => {
                skillInvocationCounts[s.skill_name] = s.total_invocations;
            });
            // Re-render skill list to show updated counts
            if (initialized) renderSkills();
        } catch (e) {
            console.error('Failed to fetch top skills:', e);
        }

        try {
            const res = await fetch('/api/mcp-servers');
            const data = await res.json();
            mcpServers = data.servers || [];
            document.getElementById('mcp-count').textContent = data.total || 0;
            renderMcpServers();
        } catch (e) {
            console.error('Failed to fetch MCP servers:', e);
        }

        document.getElementById('last-update').textContent = new Date().toLocaleTimeString();
    }

    async function loadMoreSkills() {
        if (loadingMore || skills.length >= skillsTotal) return;
        loadingMore = true;
        try {
            const res = await fetch('/api/skills?limit=' + PAGE_SIZE + '&offset=' + skills.length);
            const data = await res.json();
            const baseIdx = skills.length;
            const newItems = (data.items || []).map((s, i) => ({ ...s, _idx: baseIdx + i }));
            if (newItems.length > 0) {
                skills = skills.concat(newItems);
                if (sortOrder === 'alpha') {
                    skills.sort((a, b) => a.name.localeCompare(b.name));
                    renderSkills();
                } else {
                    appendSkills(newItems);
                }
            }
        } catch (e) {
            console.error('Failed to load more skills:', e);
        }
        loadingMore = false;
    }

    function createSkillItem(skill) {
        const div = document.createElement('div');
        div.className = 'skill-item';
        div.dataset.name = skill.name;

        const topRow = document.createElement('div');
        topRow.className = 'skill-top-row';

        const nameSpan = document.createElement('span');
        nameSpan.className = 'skill-name';
        nameSpan.textContent = skill.name;

        topRow.appendChild(nameSpan);

        const count = skillInvocationCounts[skill.name];
        if (count > 0) {
            const badge = document.createElement('span');
            badge.className = 'skill-invocations';
            badge.textContent = count;
            badge.title = count + ' invocation' + (count !== 1 ? 's' : '');
            topRow.appendChild(badge);
        }

        const sourceSpan = document.createElement('span');
        sourceSpan.className = 'skill-source';
        sourceSpan.textContent = skill.source;

        div.appendChild(topRow);
        div.appendChild(sourceSpan);

        div.addEventListener('click', () => {
            selectedSkill = skill;
            renderMetrics();
        });

        return div;
    }

    function renderSkills() {
        const list = document.getElementById('skill-list');
        // Preserve the sentinel if it exists
        const sentinel = document.getElementById('skill-sentinel');
        list.replaceChildren();

        if (skills.length === 0) {
            const empty = document.createElement('div');
            empty.className = 'empty';
            empty.textContent = 'No skills found';
            list.appendChild(empty);
        } else {
            skills.forEach(skill => {
                list.appendChild(createSkillItem(skill));
            });
        }

        // Re-append sentinel so it stays at the bottom
        if (sentinel) list.appendChild(sentinel);
    }

    function appendSkills(newSkills) {
        const list = document.getElementById('skill-list');
        // Remove "No skills found" placeholder if present
        const empty = list.querySelector('.empty');
        if (empty) empty.remove();
        newSkills.forEach(skill => {
            list.appendChild(createSkillItem(skill));
        });
    }

    function eventDetail(event) {
        // No escapeHtml needed — callers set this via .textContent which is XSS-safe
        if (event.type === 'SkillInvocation') {
            return (event.skill_name || '') + ' - ' + (event.success ? 'OK' : 'FAIL');
        }
        if (event.type === 'Validation') {
            return (event.skill_name || '') + ' - ' + (event.checks_failed?.length ? 'FAIL' : 'PASS');
        }
        if (event.type === 'Sync') {
            return (event.operation || '') + ' - ' + (event.status || '');
        }
        if (event.type === 'RuleTrigger') {
            return (event.rule_name || '') + ' - ' + (event.outcome || '');
        }
        // Fallback: strip unique fields (id, created_at) so dedup keys match
        var copy = Object.assign({}, event);
        delete copy.id;
        delete copy.created_at;
        delete copy.type;
        return JSON.stringify(copy).slice(0, 80);
    }

    /** Extract a short HH:MM:SS timestamp from the event's created_at field. */
    function formatTimestamp(event) {
        const raw = event.created_at;
        if (!raw) return '';
        try {
            const d = new Date(raw);
            if (isNaN(d.getTime())) return raw.slice(11, 19) || raw;
            return d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' });
        } catch (_) {
            return raw;
        }
    }

    function createEventItem(event, count) {
        const div = document.createElement('div');
        div.className = 'activity-item ' + (event.type || '');

        const tsSpan = document.createElement('span');
        tsSpan.className = 'event-timestamp';
        tsSpan.textContent = formatTimestamp(event);

        const typeSpan = document.createElement('span');
        typeSpan.className = 'event-type';
        typeSpan.textContent = event.type || 'Event';

        const detailSpan = document.createElement('span');
        detailSpan.className = 'event-detail';
        detailSpan.textContent = eventDetail(event);

        div.appendChild(tsSpan);
        div.appendChild(typeSpan);
        div.appendChild(detailSpan);

        if (count > 1) {
            const countBadge = document.createElement('span');
            countBadge.className = 'event-count';
            countBadge.textContent = '\u00d7' + count;
            countBadge.title = count + ' identical events';
            div.appendChild(countBadge);
        }

        return div;
    }

    /** Collapse ALL events with identical type+detail into groups.
     *  Groups are ordered by the most recent event (latest timestamp wins). */
    function deduplicateEvents(rawEvents) {
        const map = new Map();   // key -> { event, count }
        const order = [];        // insertion order of keys
        for (const event of rawEvents) {
            const key = (event.type || '') + ':' + eventDetail(event);
            if (map.has(key)) {
                const group = map.get(key);
                group.count += 1;
                // Keep the most recent event (last seen = latest timestamp)
                group.event = event;
            } else {
                const group = { event, key, count: 1 };
                map.set(key, group);
                order.push(key);
            }
        }
        return order.map(k => map.get(k));
    }

    function renderEvents() {
        const list = document.getElementById('activity-list');
        list.replaceChildren();

        if (events.length === 0) {
            const empty = document.createElement('div');
            empty.className = 'empty';
            empty.textContent = 'No recent activity';
            list.appendChild(empty);
            return;
        }

        const groups = deduplicateEvents(events);
        groups.slice(0, 50).forEach(({ event, count }) => {
            list.appendChild(createEventItem(event, count));
        });
    }

    function addDlEntry(dl, label, value, className) {
        const dt = document.createElement('dt');
        dt.textContent = label;
        const dd = document.createElement('dd');
        dd.textContent = value;
        if (className) dd.className = className;
        dl.appendChild(dt);
        dl.appendChild(dd);
    }

    function renderMetrics() {
        const content = document.getElementById('metrics-content');
        content.replaceChildren();

        if (!selectedSkill) {
            const empty = document.createElement('div');
            empty.className = 'empty';
            empty.textContent = 'Select a skill to view details';
            content.appendChild(empty);
            return;
        }

        const h3 = document.createElement('h3');
        h3.textContent = selectedSkill.name;
        content.appendChild(h3);

        const dl = document.createElement('dl');
        addDlEntry(dl, 'Path', selectedSkill.path);
        addDlEntry(dl, 'Source', selectedSkill.source);
        addDlEntry(dl, 'Description', selectedSkill.description || 'N/A');
        content.appendChild(dl);

        // Fetch and display invocation stats
        fetch('/api/metrics/skills/' + encodeURIComponent(selectedSkill.name))
            .then(res => res.json())
            .then(data => {
                // Guard: user may have clicked a different skill
                if (!selectedSkill || selectedSkill.name !== data.skill) return;

                const statsDl = document.createElement('dl');
                statsDl.className = 'metrics-stats';
                addDlEntry(statsDl, 'Invocations', data.total_invocations);
                addDlEntry(statsDl, 'Successful', data.successful_invocations, 'stat-success');
                addDlEntry(statsDl, 'Failed', data.failed_invocations, 'stat-error');
                if (data.total_invocations > 0) {
                    const rate = ((data.successful_invocations / data.total_invocations) * 100).toFixed(1);
                    addDlEntry(statsDl, 'Success Rate', rate + '%');
                    addDlEntry(statsDl, 'Avg Duration', data.avg_duration_ms.toFixed(1) + ' ms');
                }
                if (data.total_tokens > 0) {
                    addDlEntry(statsDl, 'Total Tokens', data.total_tokens.toLocaleString());
                }
                content.appendChild(statsDl);
            })
            .catch(e => console.error('Failed to fetch skill stats:', e));
    }

    // Sentinel-based infinite scroll using IntersectionObserver.
    // Works regardless of which ancestor container actually scrolls.
    function setupInfiniteScroll() {
        const list = document.getElementById('skill-list');
        if (!list) return;

        const sentinel = document.createElement('div');
        sentinel.id = 'skill-sentinel';
        sentinel.style.height = '1px';
        list.appendChild(sentinel);

        const observer = new IntersectionObserver((entries) => {
            if (entries[0].isIntersecting && skills.length < skillsTotal) {
                loadMoreSkills();
            }
        }, { threshold: 0 });

        observer.observe(sentinel);
    }

    function toggleSort() {
        sortOrder = sortOrder === 'discovery' ? 'alpha' : 'discovery';
        if (sortOrder === 'alpha') {
            skills.sort((a, b) => a.name.localeCompare(b.name));
        } else {
            // Restore discovery order by re-sorting on the original index
            skills.sort((a, b) => (a._idx ?? 0) - (b._idx ?? 0));
        }
        const btn = document.getElementById('sort-btn');
        if (btn) btn.textContent = sortOrder === 'alpha' ? 'Sort: A-Z' : 'Sort: Discovery';
        renderSkills();
    }

    function renderMcpServers() {
        const list = document.getElementById('mcp-list');
        if (!list) return;
        list.replaceChildren();

        if (mcpServers.length === 0) {
            const empty = document.createElement('div');
            empty.className = 'empty';
            empty.textContent = 'No MCP servers found';
            list.appendChild(empty);
            return;
        }

        mcpServers.forEach(server => {
            const div = document.createElement('div');
            div.className = 'mcp-server-item';

            const header = document.createElement('div');
            header.className = 'mcp-server-header';

            const nameSpan = document.createElement('span');
            nameSpan.className = 'mcp-server-name';
            nameSpan.textContent = server.name;

            const sourceBadge = document.createElement('span');
            sourceBadge.className = 'mcp-server-source';
            sourceBadge.textContent = server.source;

            const transportBadge = document.createElement('span');
            transportBadge.className = 'mcp-server-transport';
            transportBadge.textContent = server.transport;

            header.appendChild(nameSpan);
            header.appendChild(sourceBadge);
            header.appendChild(transportBadge);

            if (!server.enabled) {
                const disabledBadge = document.createElement('span');
                disabledBadge.className = 'mcp-server-disabled';
                disabledBadge.textContent = 'disabled';
                header.appendChild(disabledBadge);
            }

            div.appendChild(header);

            const cmdDiv = document.createElement('div');
            cmdDiv.className = 'mcp-server-cmd';
            cmdDiv.textContent = server.command + (server.args.length ? ' ' + server.args.join(' ') : '');
            div.appendChild(cmdDiv);

            if (server.allowed_tools.length > 0) {
                const allowed = document.createElement('div');
                allowed.className = 'mcp-server-tools mcp-tools-allowed';
                allowed.textContent = 'Allowed: ' + server.allowed_tools.join(', ');
                div.appendChild(allowed);
            }

            if (server.disabled_tools.length > 0) {
                const disabled = document.createElement('div');
                disabled.className = 'mcp-server-tools mcp-tools-disabled';
                disabled.textContent = 'Disabled: ' + server.disabled_tools.join(', ');
                div.appendChild(disabled);
            }

            list.appendChild(div);
        });
    }

    // Initialize once
    function init() {
        refresh().then(setupInfiniteScroll);
        setInterval(refresh, 30000);

        const sortBtn = document.getElementById('sort-btn');
        if (sortBtn) sortBtn.addEventListener('click', toggleSort);
    }

    if (document.readyState === 'loading') {
        document.addEventListener('DOMContentLoaded', init);
    } else {
        init();
    }
})();
