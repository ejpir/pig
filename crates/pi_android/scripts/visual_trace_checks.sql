-- Timing diagnostics only. No query below turns ticks or display-only rows
-- into this NativeActivity app's presentation FPS.
SELECT name, value FROM stats
WHERE value != 0 AND
 (name GLOB '*overrun*' OR name GLOB '*overwrite*' OR name GLOB '*discard*'
  OR name GLOB '*loss*' OR name GLOB '*error*' OR name GLOB '*drop*'
  OR name GLOB '*wrap*') ORDER BY name;

-- Validate actual scheduler retention, independently of metadata trace_bounds.
SELECT ROUND((MAX(ts+dur)-MIN(ts))/1e9,3) AS scheduler_coverage_s,
       ROUND(MIN(ts)/1e9,3) AS first_s,ROUND(MAX(ts+dur)/1e9,3) AS last_s
FROM sched WHERE dur>0;

SELECT p.pid,p.name,t.tid,t.name AS thread_name,t.utid
FROM process p JOIN thread t USING(upid)
WHERE p.name IN ('dev.pi.android','dev.pi.android.uitest')
 AND t.name='android_main'
 AND EXISTS (SELECT 1 FROM slice s JOIN thread_track tt ON tt.id=s.track_id
             WHERE tt.utid=t.utid AND s.name='pi.main.poll');

SELECT s.name,COUNT(*) AS calls,ROUND(AVG(CASE WHEN s.dur>=0 THEN s.dur END)/1e6,3) AS mean_ms,
       ROUND(MAX(s.dur)/1e6,3) AS max_ms,SUM(s.dur<0) AS incomplete
FROM slice s JOIN thread_track tt ON tt.id=s.track_id JOIN thread t USING(utid)
JOIN process p USING(upid)
WHERE p.name IN ('dev.pi.android','dev.pi.android.uitest') AND s.name GLOB 'pi.*'
GROUP BY s.name ORDER BY s.name;

-- Labelled sleep overlap: scoped CPU calls can include driver/fence blocking.
-- These are inclusive nested spans; never sum the rows as exclusive attribution.
SELECT s.name,ROUND(SUM(MIN(st.ts+st.dur,s.ts+s.dur)-MAX(st.ts,s.ts))/1e6,3) AS sleep_overlap_ms
FROM slice s JOIN thread_track tt ON tt.id=s.track_id
JOIN thread t USING(utid) JOIN process p USING(upid)
JOIN thread_state st ON st.utid=t.utid AND st.dur>0 AND st.state IN ('S','D')
 AND st.ts<s.ts+s.dur AND st.ts+st.dur>s.ts
WHERE p.name IN ('dev.pi.android','dev.pi.android.uitest') AND s.dur>0
 AND s.name IN ('pi.main.poll','pi.main.tasks','pi.gpu.acquire','pi.gpu.encode_submit','pi.gpu.present_call')
GROUP BY s.name ORDER BY s.name;

-- Verify whether any app-specific SURFACE timeline is actually present.
SELECT p.name,f.layer_name,COUNT(*) AS frames,
       SUM(f.surface_frame_token IS NOT NULL) AS surface_rows
FROM actual_frame_timeline_slice f LEFT JOIN process p USING(upid)
GROUP BY p.name,f.layer_name ORDER BY frames DESC;
