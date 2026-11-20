package api

import (
	"database/sql"
	"net/http"
	"time"

	"github.com/gin-gonic/gin"

	"stock-predictor/internal/apikeys"
)

// The keys that open /mcp. This is the only place a key is minted, and the only place its
// plaintext exists: the create response returns it once and nothing can fetch it again.

func listKeys(c *gin.Context, st *AppState) {
	keys, err := apikeys.List(st.Pool)
	if err != nil {
		respondErr(c, internalErr(err))
		return
	}
	c.JSON(http.StatusOK, gin.H{"keys": keys})
}

func createKey(c *gin.Context, st *AppState) {
	var body struct {
		Name  string `json:"name"`
		Scope string `json:"scope"`
		Days  *int   `json:"days"`
	}
	if err := c.ShouldBindJSON(&body); err != nil {
		respondErr(c, badRequest("invalid body"))
		return
	}
	scope := apikeys.Scope(body.Scope)
	token, key, err := apikeys.New(st.Pool, body.Name, scope, body.Days, time.Now().UTC())
	if err != nil {
		respondErr(c, badRequest(err.Error()))
		return
	}
	c.JSON(http.StatusCreated, gin.H{"key": key, "token": token})
}

func revokeKey(c *gin.Context, st *AppState) {
	id := c.Param("id")
	revoked, err := apikeys.Revoke(st.Pool, id, time.Now().UTC())
	if err != nil {
		respondErr(c, internalErr(err))
		return
	}
	if !revoked {
		respondErr(c, notFound("no live key with that id"))
		return
	}
	c.JSON(http.StatusOK, gin.H{"id": id, "revoked": true})
}

// verifyKey is the MCP middleware's question, kept here so the REST and MCP paths share one rule.
func verifyKey(pool *sql.DB, token string) (*apikeys.Key, error) {
	key, err := apikeys.Verify(pool, token, time.Now().UTC())
	if err != nil {
		return nil, err
	}
	apikeys.Touch(pool, key, time.Now().UTC())
	return key, nil
}
