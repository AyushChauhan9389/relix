# Relix Web Helper

A Chrome / Edge extension for the Relix web app. Debrid CDNs and many file hosts don't let web pages
read their files (CORS), so the web player can't repackage MKV or convert DTS/AC-3 audio from them.
This adds the permission header to responses **only for requests the Relix page makes**
(`initiatorDomains: relix.carbonkit.tech`); it reads nothing and changes nothing else.

Install: unzip `relix-web-helper.zip`, open `chrome://extensions`, turn on Developer mode, **Load unpacked**
and pick the folder.
