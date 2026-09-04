import QtQuick
import Quickshell
import Quickshell.Io
import qs.Ui

BarWidget {
  id: root
  moduleName: "vitor.perfo"
  property var manifest: null
  property var snapshot: null
  readonly property string binaryPath: {
    if (manifest && manifest.__sourceDir)
      return String(manifest.__sourceDir) + "/bin/perfo"
    return Quickshell.env("PERFO_BIN") || (Quickshell.env("HOME") + "/.local/bin/perfo")
  }

  readonly property string cpuLabel: snapshot ? "C " + Math.round(snapshot.overall_percent) + "%" : "C --"
  readonly property string memLabel: snapshot && snapshot.total_mem_bytes > 0
    ? "M " + Math.round(snapshot.used_mem_bytes * 100 / snapshot.total_mem_bytes) + "%"
    : "M --"
  readonly property string label: cpuLabel + "  " + memLabel
  readonly property bool opened: panelLoader.item ? panelLoader.item.opened === true : false

  implicitWidth: root.vertical ? root.barSize : 112
  implicitHeight: root.barSize

  function injectPanel() {
    if (!panelLoader.item) return
    panelLoader.item.bar = root.bar
    panelLoader.item.anchorItem = button
    panelLoader.item.hostWidget = root
    panelLoader.item.snapshot = root.snapshot
  }

  function open() {
    if (panelLoader.item) panelLoader.item.open()
  }

  function close() {
    if (panelLoader.item) panelLoader.item.close()
  }

  function toggle() {
    if (root.opened) root.close()
    else root.open()
  }

  onSnapshotChanged: injectPanel()
  onBarChanged: injectPanel()

  Process {
    id: collector
    command: [root.binaryPath, "stream", "--json"]
    running: true
    stdout: SplitParser {
      onRead: function(line) {
        try {
          root.snapshot = JSON.parse(line)
        } catch (error) {
          console.warn("vitor.perfo: invalid JSON snapshot", error)
        }
      }
    }
  }

  IpcHandler {
    target: "vitor.perfo"
    function open(): void { root.open() }
    function close(): void { root.close() }
    function show(): void { root.open() }
    function hide(): void { root.close() }
    function toggle(): void { root.toggle() }
  }

  Loader {
    id: panelLoader
    active: true
    source: Qt.resolvedUrl("Panel.qml")
    visible: false
    onLoaded: root.injectPanel()
  }

  Item {
    id: button
    anchors.fill: parent
    anchors.leftMargin: 6
    anchors.rightMargin: 6

    Text {
      anchors.fill: parent
      text: root.label
      color: root.bar ? root.bar.barForeground : "white"
      font.family: root.bar ? root.bar.fontFamily : "monospace"
      font.pixelSize: 13
      horizontalAlignment: Text.AlignHCenter
      verticalAlignment: Text.AlignVCenter
      elide: Text.ElideRight
    }

    MouseArea {
      anchors.fill: parent
      acceptedButtons: Qt.LeftButton | Qt.RightButton
      onClicked: function(mouse) {
        if (mouse.button === Qt.LeftButton) root.toggle()
        else if (mouse.button === Qt.RightButton && panelLoader.item) panelLoader.item.openTerminal()
      }
      onEntered: if (root.bar) root.bar.showTooltip(root, "Perfo: left click for metrics, right click for full TUI")
      onExited: if (root.bar) root.bar.hideTooltip(root)
    }
  }
}
