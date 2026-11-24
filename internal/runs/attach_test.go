package runs

import "testing"

// A manual run is closed by the caller, not by the plan that filled it in: Attach is what keeps the
// conversation when that closing Finish passes nil.
func TestAttachSurvivesAFinishThatPassesNoConversation(t *testing.T) {
	p := pool(t)
	id, err := Start(p, PlanStock, TriggerManual, nil)
	if err != nil {
		t.Fatal(err)
	}
	if err := Attach(p, id, "plan-1795548318346"); err != nil {
		t.Fatal(err)
	}
	if err := Finish(p, id, "nothing worth doing — the model passed", nil); err != nil {
		t.Fatal(err)
	}
	rows, err := Recent(p, 10)
	if err != nil || len(rows) != 1 {
		t.Fatalf("%d rows, %v", len(rows), err)
	}
	if rows[0].ConversationID == nil || *rows[0].ConversationID != "plan-1795548318346" {
		t.Fatalf("conversation: %v", rows[0].ConversationID)
	}
}
