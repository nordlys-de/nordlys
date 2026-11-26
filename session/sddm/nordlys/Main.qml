// The Nordlys login screen for SDDM (Qt 6): the aurora wallpaper, a clock,
// and a frosted card with the user, password and session.
//
// Try it without logging out:
//   sddm-greeter-qt6 --test-mode --theme session/sddm/nordlys

pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts

Rectangle {
    id: root
    width: 1920
    height: 1080
    color: "#0B0E14"

    readonly property color accent: config.accent ? config.accent : "#8AB4FF"
    readonly property color ink: "#E8ECF4"
    readonly property color muted: "#A3ACBD"
    readonly property color glass: "#C7161A24"
    readonly property color field: "#14FFFFFF"
    readonly property color line: "#24FFFFFF"
    readonly property string sans: "Inter"

    function login() {
        message.text = ""
        sddm.login(user.text, password.text, session.currentIndex)
    }

    Image {
        anchors.fill: parent
        source: config.background ? config.background : "wallpaper.jpg"
        fillMode: Image.PreserveAspectCrop
        asynchronous: true
    }
    // Darker toward the bottom, where the card sits.
    Rectangle {
        anchors.fill: parent
        gradient: Gradient {
            GradientStop { position: 0.0; color: "#33080A10" }
            GradientStop { position: 1.0; color: "#99080A10" }
        }
    }

    Column {
        id: clock
        anchors.horizontalCenter: parent.horizontalCenter
        y: root.height * 0.16
        spacing: 4
        Text {
            id: time
            anchors.horizontalCenter: parent.horizontalCenter
            color: root.ink
            font.family: root.sans
            font.pixelSize: Math.round(root.height * 0.09)
            font.weight: Font.Light
        }
        Text {
            id: day
            anchors.horizontalCenter: parent.horizontalCenter
            color: root.muted
            font.family: root.sans
            font.pixelSize: 18
        }
        function tick() {
            const now = new Date()
            time.text = Qt.formatTime(now, "HH:mm")
            day.text = Qt.formatDate(now, "dddd d MMMM")
        }
        Component.onCompleted: tick()
    }
    Timer {
        interval: 1000
        running: true
        repeat: true
        onTriggered: clock.tick()
    }

    Rectangle {
        id: card
        width: 380
        height: form.implicitHeight + 56
        anchors.horizontalCenter: parent.horizontalCenter
        y: Math.max(clock.y + clock.height + 48, root.height * 0.46)
        radius: 18
        color: root.glass
        border.color: root.line
        border.width: 1

        ColumnLayout {
            id: form
            anchors.fill: parent
            anchors.margins: 28
            spacing: 12

            Rectangle {
                Layout.alignment: Qt.AlignHCenter
                Layout.preferredWidth: 64
                Layout.preferredHeight: 64
                radius: 32
                color: root.accent
                Text {
                    anchors.centerIn: parent
                    text: user.text.length > 0 ? user.text.charAt(0).toUpperCase() : "?"
                    color: "#0B0E14"
                    font.family: root.sans
                    font.pixelSize: 28
                    font.weight: Font.DemiBold
                }
            }

            Text {
                Layout.alignment: Qt.AlignHCenter
                text: sddm.hostName
                color: root.muted
                font.family: root.sans
                font.pixelSize: 13
            }

            TextField {
                id: user
                Layout.fillWidth: true
                text: userModel.lastUser
                placeholderText: "User"
                color: root.ink
                placeholderTextColor: root.muted
                font.family: root.sans
                font.pixelSize: 15
                padding: 10
                background: Rectangle {
                    radius: 10
                    color: root.field
                    border.color: user.activeFocus ? root.accent : root.line
                }
                KeyNavigation.tab: password
                onAccepted: password.forceActiveFocus()
            }

            TextField {
                id: password
                Layout.fillWidth: true
                echoMode: TextInput.Password
                placeholderText: "Password"
                color: root.ink
                placeholderTextColor: root.muted
                font.family: root.sans
                font.pixelSize: 15
                padding: 10
                focus: true
                background: Rectangle {
                    radius: 10
                    color: root.field
                    border.color: password.activeFocus ? root.accent : root.line
                }
                KeyNavigation.tab: session
                onAccepted: root.login()
            }

            Text {
                id: message
                Layout.fillWidth: true
                visible: text.length > 0
                color: "#FF8A8A"
                font.family: root.sans
                font.pixelSize: 13
                wrapMode: Text.Wrap
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: 10

                ComboBox {
                    id: session
                    Layout.fillWidth: true
                    model: sessionModel
                    textRole: "name"
                    currentIndex: sessionModel.lastIndex
                    font.family: root.sans
                    font.pixelSize: 13
                    contentItem: Text {
                        leftPadding: 10
                        text: session.displayText
                        color: root.ink
                        font: session.font
                        verticalAlignment: Text.AlignVCenter
                        elide: Text.ElideRight
                    }
                    background: Rectangle {
                        implicitHeight: 38
                        radius: 10
                        color: root.field
                        border.color: session.activeFocus ? root.accent : root.line
                    }
                    delegate: ItemDelegate {
                        required property int index
                        required property string name
                        width: session.width
                        contentItem: Text {
                            text: name
                            color: root.ink
                            font: session.font
                        }
                        background: Rectangle {
                            color: highlighted ? root.line : "transparent"
                        }
                        highlighted: session.highlightedIndex === index
                    }
                    popup.background: Rectangle {
                        radius: 10
                        color: "#F0161A24"
                        border.color: root.line
                    }
                    KeyNavigation.tab: loginButton
                }

                Button {
                    id: loginButton
                    text: "Log in"
                    font.family: root.sans
                    font.pixelSize: 14
                    font.weight: Font.DemiBold
                    onClicked: root.login()
                    contentItem: Text {
                        text: loginButton.text
                        font: loginButton.font
                        color: "#0B0E14"
                        horizontalAlignment: Text.AlignHCenter
                        verticalAlignment: Text.AlignVCenter
                    }
                    background: Rectangle {
                        implicitWidth: 96
                        implicitHeight: 38
                        radius: 10
                        color: loginButton.down || loginButton.hovered ? Qt.lighter(root.accent, 1.1) : root.accent
                    }
                    KeyNavigation.tab: user
                }
            }
        }
    }

    // Suspend, restart and shut down, bottom right.
    Row {
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        anchors.margins: 28
        spacing: 8

        Repeater {
            model: [
                { label: "Suspend", can: sddm.canSuspend, act: function() { sddm.suspend() } },
                { label: "Restart", can: sddm.canReboot, act: function() { sddm.reboot() } },
                { label: "Shut down", can: sddm.canPowerOff, act: function() { sddm.powerOff() } }
            ]
            delegate: Button {
                id: power
                required property var modelData
                visible: modelData.can
                text: modelData.label
                font.family: root.sans
                font.pixelSize: 13
                onClicked: modelData.act()
                contentItem: Text {
                    text: power.text
                    font: power.font
                    color: root.ink
                    horizontalAlignment: Text.AlignHCenter
                    verticalAlignment: Text.AlignVCenter
                }
                background: Rectangle {
                    implicitWidth: 92
                    implicitHeight: 34
                    radius: 9
                    color: power.hovered ? "#33FFFFFF" : "#1AFFFFFF"
                    border.color: root.line
                }
            }
        }
    }

    Connections {
        target: sddm
        function onLoginFailed() {
            message.text = "That password didn't work. Try again."
            password.text = ""
            password.forceActiveFocus()
        }
    }

    Component.onCompleted: {
        if (user.text.length === 0)
            user.forceActiveFocus()
        else
            password.forceActiveFocus()
    }
}
