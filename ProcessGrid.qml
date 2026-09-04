import QtQuick
import qs.Commons

Grid {
  id: processGrid

  property var processes: []
  property color foreground: "white"
  property string fontFamily: "monospace"
  property int processCpuWidth: 52

  function processName(command, pid) {
    var executable = String(command || "").trim().split(/\s+/)[0]
    if (!executable) return String(pid)
    var slash = executable.lastIndexOf("/")
    if (slash >= 0) executable = executable.slice(slash + 1)
    executable = executable.replace(/^["']+|["']+$/g, "")
    return executable || String(pid)
  }

  Repeater {
    model: processGrid.processes
    delegate: Row {
      width: (processGrid.width - processGrid.columnSpacing) / 2

      Text {
        width: parent.width - processGrid.processCpuWidth
        text: processGrid.processName(modelData.name || modelData.cmd, modelData.pid)
        color: processGrid.foreground
        font.family: processGrid.fontFamily
        font.pixelSize: Style.font.bodySmall
        elide: Text.ElideRight
      }

      Text {
        width: processGrid.processCpuWidth
        text: Math.round(Number(modelData.cpu_percent) || 0) + "%"
        color: processGrid.foreground
        font.family: processGrid.fontFamily
        font.pixelSize: Style.font.bodySmall
        horizontalAlignment: Text.AlignRight
      }
    }
  }
}
