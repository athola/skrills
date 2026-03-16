// Skrills Dashboard Client-Side Logic
(function() {
    let skills = [];
    let events = [];
    let selectedSkill = null;
    let skillsTotal = 0;
    let loadingMore = false;
    const PAGE_SIZE = 50;

    let initialized = false;
    let sortOrder = 'discovery'; // 'discovery' | 'alpha'

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

        const nameSpan = document.createElement('span');
        nameSpan.className = 'skill-name';
        nameSpan.textContent = skill.name;

        const sourceSpan = document.createElement('span');
        sourceSpan.className = 'skill-source';
        sourceSpan.textContent = skill.source;

        div.appendChild(nameSpan);
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
        return JSON.stringify(event).slice(0, 50);
    }

    function createEventItem(event) {
        const div = document.createElement('div');
        div.className = 'activity-item ' + (event.type || '');

        const typeSpan = document.createElement('span');
        typeSpan.className = 'event-type';
        typeSpan.textContent = event.type || 'Event';

        const detailSpan = document.createElement('span');
        detailSpan.className = 'event-detail';
        detailSpan.textContent = eventDetail(event);

        div.appendChild(typeSpan);
        div.appendChild(detailSpan);

        return div;
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

        events.slice(0, 50).forEach(event => {
            list.appendChild(createEventItem(event));
        });
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

        const dtPath = document.createElement('dt');
        dtPath.textContent = 'Path';
        const ddPath = document.createElement('dd');
        ddPath.textContent = selectedSkill.path;

        const dtSource = document.createElement('dt');
        dtSource.textContent = 'Source';
        const ddSource = document.createElement('dd');
        ddSource.textContent = selectedSkill.source;

        const dtDesc = document.createElement('dt');
        dtDesc.textContent = 'Description';
        const ddDesc = document.createElement('dd');
        ddDesc.textContent = selectedSkill.description || 'N/A';

        dl.appendChild(dtPath);
        dl.appendChild(ddPath);
        dl.appendChild(dtSource);
        dl.appendChild(ddSource);
        dl.appendChild(dtDesc);
        dl.appendChild(ddDesc);

        content.appendChild(dl);
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
