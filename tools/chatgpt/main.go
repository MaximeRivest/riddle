package main

import (
	"crypto/rand"
	"encoding/hex"
	"flag"
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

func main() {
	if len(os.Args) < 2 {
		fmt.Fprintln(os.Stderr, "Usage: riddle-chatgpt login|serve|status --state-dir DIRECTORY")
		os.Exit(2)
	}
	flags := flag.NewFlagSet(os.Args[1], flag.ExitOnError)
	state := flags.String("state-dir", "", "directory for this Riddle installation's credentials")
	listen := flags.String("listen", "127.0.0.1:17843", "loopback HTTP listener")
	model := flags.String("model", "", "ChatGPT model slug; default uses account catalog")
	flags.Parse(os.Args[2:])
	if *state == "" {
		fatal(fmt.Errorf("--state-dir is required"))
	}
	if err := os.MkdirAll(*state, 0700); err != nil {
		fatal(err)
	}
	var err error
	switch os.Args[1] {
	case "login":
		err = login(*state)
	case "serve":
		var token string
		token, err = localToken(*state)
		if err == nil {
			err = serveRelay(*state, *listen, token, *model)
		}
	case "status":
		var c Credentials
		c, err = loadCredentials(*state)
		if err == nil {
			fmt.Printf("ChatGPT sign-in saved; plan permission: %t\n", strings.Contains(" "+c.Scope+" ", " chatgpt.tokens.use.direct "))
		}
	default:
		err = fmt.Errorf("unknown command %q", os.Args[1])
	}
	if err != nil {
		fatal(err)
	}
}

func localToken(stateDir string) (string, error) {
	path := filepath.Join(stateDir, "local-token")
	data, err := os.ReadFile(path)
	if err == nil {
		value := strings.TrimSpace(string(data))
		if len(value) < 32 {
			return "", fmt.Errorf("invalid local relay token")
		}
		return value, nil
	}
	if !os.IsNotExist(err) {
		return "", err
	}
	bytes := make([]byte, 32)
	if _, err = rand.Read(bytes); err != nil {
		return "", err
	}
	value := hex.EncodeToString(bytes)
	file, err := os.OpenFile(path, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0600)
	if err != nil {
		return "", err
	}
	_, err = file.WriteString(value + "\n")
	closeErr := file.Close()
	if err == nil {
		err = closeErr
	}
	return value, err
}

func fatal(err error) {
	fmt.Fprintln(os.Stderr, "riddle-chatgpt:", err)
	os.Exit(1)
}
