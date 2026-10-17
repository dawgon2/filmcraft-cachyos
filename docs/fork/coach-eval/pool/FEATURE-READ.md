# FEATURE-POOL training pairs: owner read (Q35 rule)

35 of the 80 FEATURE-POOL training pairs (`pool/feature-*.jsonl`), chosen at random (Python `random.Random(20261016).sample`, so the choice can be repeated), for the owner to read before any tune uses FEATURE-POOL pairs (FORK-PLAN §1a.100, OWNER-QUESTIONS Q52). Rewritten whenever the pool grows a lot; this read covers rows S1.1–S1.5, AI5 and AI4.

What to look for: steps that would not work in Revolution, invented controls, wrong numbers, gaps not admitted, and answers that read as one right way where options would serve better. Note anything in OWNER-QUESTIONS Q52; the lane fixes the pair and re-checks its batch.

## 1. Can a script open the GitHub page without consent?

No: `coach.shareExample` with route github needs `consent: true` and then returns the `url` of the pre-filled issue page. route preview returns the text, route file with a folder saves it; neither needs consent.

*Sources:* `color.coach.assistant-share` · *kind:* `feature` · *from:* feature-assistant-share.jsonl line 7

## 2. Where exactly does the package end up?

In the folder you pick, as `<destination>/<project name> package/` holding the project file, every media file it uses and `manifest.json` (and the used custom shader effects in `Effects/`). With the one-file option it is a single `<project name>.fcpackage` there instead.

*Sources:* `network-storage.package` · *kind:* `feature` · *from:* feature-project-package.jsonl line 5

## 3. Does it check the file afterwards too?

> *Follows:* Does Revolution do anything special when the project file is on a server?

Yes. After the save, Revolution reads the file on the share back and compares it with what it saved; if it differs, the save reports an error ending with This save is kept on this computer: and the local copy's path, so you can carry on from that copy.

*Sources:* `network-storage.saving` · *kind:* `follow-up` · *from:* feature-network-storage-saving.jsonl line 8

## 4. Does the network timeout make my local media slower?

No. The 8-second limit and the 30-second share-down rule apply only to files on network shares; files on local disks are read exactly as before.

*Sources:* `network-storage.drops` · *kind:* `feature` · *from:* feature-network-storage-drops.jsonl line 5

## 5. Can I cancel a package that's taking ages?

Yes. Packaging runs in the background with progress in the status bar and can be stopped. When you stop it, the half-made package folder is removed, and an existing package folder is never overwritten.

*Sources:* `network-storage.package` · *kind:* `feature` · *from:* feature-project-package.jsonl line 4

## 6. How do I change how much space network copies may take?

Settings ▸ Media Cache ▸ Network Copies ▸ Local copies size limit (`mediaCache.networkCopiesGb`, 100 GB by default). Above it the copies used least recently are deleted, the open project's last. Revolution only ever deletes copies it made.

*Sources:* `network-storage.copies` · *kind:* `feature` · *from:* feature-network-storage-copies.jsonl line 3

## 7. And will the app hang meanwhile?

> *Follows:* What happens to my clips if the home server reboots during an edit?

No. Each read, open or size check on the share gets 8 seconds; after that the clip shows as offline and Revolution treats the whole share as down for 30 seconds, so nothing waits on the dead server. Still being built: the last few checks the interface does itself (audio waveforms, the Link Media preview) are being moved to background threads.

*Sources:* `network-storage.drops` · *kind:* `follow-up` · *from:* feature-network-storage-drops.jsonl line 9

## 8. How do I check whether my project depends on a server?

Open File ▸ Project Settings ▸ General (`file.projectSettings.general`). The first line says Project file: on a local disk, or Project file: on a network share with the share's type and server, e.g. (cifs, //nas/media) (`projectSettings.storage.project`). If any clips are on shares, the next line counts them: Clips on network shares: 3 of 12 (`projectSettings.storage.clips`). In the Project panel (Media Pool in the Resolve workflow) each such clip also has a grey NET badge.

*Sources:* `network-storage.check` · *kind:* `feature` · *from:* feature-network-storage-check.jsonl line 1

## 9. What does Help improve the Assistant… do?

It opens a dialog to share the answer as an example or save it. The preview shows exactly the text that would be shared or saved (`coach.shareExample`): the question, the answer, the guide sections it used, your rating and note, the Revolution version and the page, with private names replaced by stand-ins like <clip> and <path>. You can then share it on GitHub (public, after ticking I agree), save it to a file, or close the dialog and do neither.

*Sources:* `color.coach.assistant-share` · *kind:* `feature` · *from:* feature-assistant-share.jsonl line 2

## 10. How do I package a project from a script?

Run `projects.package` with `path` (the project file) and `destination` (the folder to put it in). Add `"proxies": true` to include proxies, `"format": "file"` for a single `.fcpackage` file instead of a folder, or `"dryRun": true` to write nothing and just get `estimateBytes`.

*Sources:* `network-storage.package` · *kind:* `feature` · *from:* feature-project-package.jsonl line 1

## 11. Why is editing straight off a home server riskier than off a local SSD?

A mounted share looks like an ordinary folder but isn't one: reads are slower than a local disk, the share can stop answering when the server sleeps or the Wi-Fi drops, and saving over it isn't always as safe as saving locally. That's why Revolution first works out where each file lives (the NET badge, the storage lines in Project Settings ▸ General) and takes extra care only with the files on shares.

*Sources:* `network-storage.why` · *kind:* `feature` · *from:* feature-network-storage-check.jsonl line 8

## 12. Does importing a package check the files before copying them?

Yes. Revolution first checks every file against the manifest: is it there, the right size, the same checksum. That reads the whole package once, in the background. If anything is missing or damaged, the import stops, names the files and copies nothing.

*Sources:* `network-storage.package` · *kind:* `feature` · *from:* feature-project-package.jsonl line 9

## 13. so for a script, how do I get that same info?

> *Earlier:* how many of my clips are on the NAS? — Open File ▸ Project Settings ▸ General: the line Clips on network shares: 3 of 12 counts them, and each such clip has a grey NET badge in the Project panel.

Run `media.storage`. It answers for the project file and every file clip (or just one clip with `{"item": <id>}`): each gets a storage kind of local, network or unknown, plus the share's fsType and source for network ones, and `networkItems` counts the clips on shares.

*Sources:* `network-storage.check` · *kind:* `chain` · *from:* feature-network-storage-check.jsonl line 13

## 14. Does Remove also delete files I put in the picture model's folder?

No. Remove (`ai.vision.remove`) deletes the picture model and the text model prepared from it; other files you put in its folder stay.

*Sources:* `color.coach.assistant` · *kind:* `feature` · *from:* feature-assistant-picture-look.jsonl line 7

## 15. is my project name in there?

> *Earlier:* i clicked help improve the assistant, what is this preview — It's exactly the text that would be shared or saved: your question, the answer, the guide sections it used, your rating and note, the Revolution version and the page.

No. In the preview, project, bin, clip and file names, your user and computer names, folder paths, network addresses and e-mail addresses are replaced by stand-ins such as <clip> and <path>. The preview is exactly what gets shared or saved.

*Sources:* `color.coach.assistant-share` · *kind:* `chain` · *from:* feature-assistant-share.jsonl line 11

## 16. What does the Assistant know about my shot without the picture model?

With each question it is told which clips are selected and, when the selected clip has a grade, what its current node does to the frame under the playhead, measured from the picture. It also gets a short measured fact sheet of that frame before and after the grade: where the mids sit against middle grey, clipping and crushing, contrast, the colour of near-neutral areas, saturation, and how much of the frame has skin-tone colours, plus whether the clip is log footage. These are numbers turned into words; the picture model adds a look at the frame itself.

*Sources:* `color.coach.assistant-panel` · *kind:* `feature` · *from:* feature-assistant-picture-look.jsonl line 8

## 17. Which questions make the Assistant look at the picture?

Questions about the picture itself: exposure, colour, skin, sky, light, once the picture model is installed. Then Revolution renders the Program frame small and the vision model writes one or two sentences about what it sees that bears on your question (for example the sky shifts from purple to orange at the horizon, low sun); the assistant gets that line with the measured numbers. How-to questions (how do I add a node?) skip it.

*Sources:* `color.coach.assistant` · *kind:* `feature` · *from:* feature-assistant-picture-look.jsonl line 1

## 18. ok and what if I don't want it using my disk at all

> *Earlier:* why is filmcraft copying my nas files? — It keeps local copies of clips on network shares (Automatic by default) so they play smoothly and keep working if the share drops; a clip shows NET ✓ when its copy is complete.

Set Settings ▸ Media Cache ▸ Network Copies ▸ Local copies of clips on network shares (`mediaCache.networkCopies`) to Off: nothing is copied and clips always play from the share. If you only want less space used, lower Local copies size limit (`mediaCache.networkCopiesGb`) instead.

*Sources:* `network-storage.copies` · *kind:* `chain` · *from:* feature-network-storage-copies.jsonl line 13

## 19. and once it's back?

> *Earlier:* my wifi dropped and half my clips say media offline — Those clips are on a network share that stopped answering: after 8 seconds without an answer Revolution shows them offline instead of freezing, and treats that share as down for 30 seconds.

They come back online by themselves within a few seconds (Revolution looks again every few seconds and when its window comes to the front), with a message like 3 clip(s) are back online. Nothing needs relinking. If Settings ▸ Media ▸ Automatically refresh growing files is off, use Link Media (`media.linkMedia`) or reopen the project.

*Sources:* `network-storage.drops` · *kind:* `chain` · *from:* feature-network-storage-drops.jsonl line 10

## 20. How do I stop Revolution from copying files off my server?

Set Settings ▸ Media Cache ▸ Network Copies ▸ Local copies of clips on network shares (`mediaCache.networkCopies`) to Off. Then it never reads or makes copies and plays every clip from the share itself.

*Sources:* `network-storage.copies` · *kind:* `feature` · *from:* feature-network-storage-copies.jsonl line 4

## 21. Does Revolution open my media files to work out whether they are remote?

No. It looks only at the system's mount table, never at the file itself, so the answer comes at once, even while the server is down or asleep.

*Sources:* `network-storage.why` · *kind:* `feature` · *from:* feature-network-storage-check.jsonl line 9

## 22. Is camera footage recompressed when I make an .fcpackage?

No. An `.fcpackage` is a zip with every file stored as it is, so camera media isn't recompressed and nothing is re-encoded. The folder it is made from is removed afterwards.

*Sources:* `network-storage.package` · *kind:* `feature` · *from:* feature-project-package.jsonl line 6

## 23. Is the local copy of a network clip re-encoded or compressed?

No. It is byte-for-byte the same file, so the picture, the grade and exports are exactly as from the original. That's the difference from proxies or optimized media, which are re-encoded smaller files.

*Sources:* `network-storage.copies` · *kind:* `feature` · *from:* feature-network-storage-copies.jsonl line 2

## 24. I unmounted the share; how long until the badges update?

A few seconds. Revolution re-reads the system's mount table every few seconds, so a share you mount or unmount is reflected in the NET badges and in Project Settings ▸ General shortly after; nothing needs refreshing by hand.

*Sources:* `network-storage.check` · *kind:* `feature` · *from:* feature-network-storage-check.jsonl line 5

## 25. What if a file is changed on the server while it's being copied?

That copy isn't kept; the file is copied again later. A copy always belongs to one version of the file, so a changed file never plays from an out-of-date copy.

*Sources:* `network-storage.copies` · *kind:* `feature` · *from:* feature-network-storage-copies.jsonl line 8

## 26. How will I know a clip has finished copying to this computer?

A message says Local copy of A001.mov ready: it now plays from this computer, and in the Project panel the clip's badge changes from NET to NET ✓; hovering it adds …; plays from its local copy.

*Sources:* `network-storage.copies` · *kind:* `feature` · *from:* feature-network-storage-copies.jsonl line 7

## 27. The copy paused when I hit play. Is something wrong?

No, that's on purpose: copies are made in the background, one file at a time, only while you aren't playing, so they never compete with playback. The copy carries on from where it stopped a few seconds after you stop.

*Sources:* `network-storage.copies` · *kind:* `feature` · *from:* feature-network-storage-copies.jsonl line 6

## 28. What does the Project file line at the top of Project Settings mean?

It tells you where the saved project file lives: Project file: on a local disk, or Project file: on a network share followed by the share's type and server, such as (cifs, //nas/media) (`projectSettings.storage.project`). It's in File ▸ Project Settings ▸ General (`file.projectSettings.general`). A project that has never been saved shows no line.

*Sources:* `network-storage.check` · *kind:* `feature` · *from:* feature-network-storage-check.jsonl line 2

## 29. A package has a shader effect with the same file name as one of mine. Which one is kept?

Yours. On import, custom shader effects from the package's `Effects/` folder are added to your own Effects folder, but one you already have under the same file name is never replaced.

*Sources:* `network-storage.package` · *kind:* `feature` · *from:* feature-project-package.jsonl line 8

## 30. Does the network badge appear in icon view or only in list view?

Both. In the Project panel (Media Pool in the Resolve workflow) a clip whose file is on a network share shows a grey NET badge next to its other badges in list view and in icon view; hover it to see the share.

*Sources:* `network-storage.check` · *kind:* `feature` · *from:* feature-network-storage-check.jsonl line 4

## 31. How do I open a package someone sent me?

On the Project Manager home screen click Import Package (`projects.importPackage`) and choose the `.fcpackage` file, or for a package folder the `manifest.json` inside it. The project goes into the folder of the projects list you are looking at. Revolution checks every file first, then copies the package into your projects folder and points every clip at its copy, so nothing is offline.

*Sources:* `network-storage.package` · *kind:* `feature` · *from:* feature-project-package.jsonl line 3

## 32. The save error gave me a path ending in .fcproj under network-backups. What is that?

That is the local copy Revolution wrote before saving to the share; the save to the share failed, so this is your latest work. Open it with File ▸ Open (`file.open`) and carry on editing; when the share works again, save it back there (File ▸ Save As, `file.saveAs`).

*Sources:* `network-storage.saving` · *kind:* `feature` · *from:* feature-network-storage-saving.jsonl line 7

## 33. Is a Plasma network folder from Dolphin treated as a network share?

Yes. Shares mounted by Plasma (Dolphin's Network places, through kio-fuse), GNOME's gvfs, /etc/fstab or autofs all count. Revolution recognises SMB / CIFS, NFS, sshfs, rclone, kio-fuse, gvfs and a few others.

*Sources:* `network-storage.why` · *kind:* `feature` · *from:* feature-network-storage-check.jsonl line 6

## 34. Is the Media Offline from a dropped share different from a deleted file?

It looks the same: the red offline mark and the Media Offline picture. The difference is what happens next: when the share answers again, the clips come back online by themselves, without relinking.

*Sources:* `network-storage.drops` · *kind:* `feature` · *from:* feature-network-storage-drops.jsonl line 6

## 35. What doesn't go into a package?

The render cache (it is rebuilt), downloaded AI models, and proxies unless you choose them (`"proxies": true`); `manifest.json` lists these as left out on purpose. Custom shader effects the project doesn't use stay home.

*Sources:* `network-storage.package` · *kind:* `feature` · *from:* feature-project-package.jsonl line 7
