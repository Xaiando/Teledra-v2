class LegacyApp {
    constructor() {
        this.menuState = true;
        this.gameLoopRunning = false;
        this.lives = 3;
        this.heartsDisplay = '❤'.repeat(this.lives);
        this.score = 0;
    }

    startGame() {
        if (this.menuState) {
            console.error('Cannot start game while in menu state.');
            return;
        }
        this.gameLoopRunning = true;
        this.runGameLoop();
    }

    runGameLoop() {
        // Simulate game loop logic
        setInterval(() => {
            this.updateScore();
            if (this.checkForLives()) {
                this.endGame();
            }
        }, 100);
    }

    updateScore() {
        this.score += 1;
        console.log(`Score: ${this.score}`);
    }

    checkForLives() {
        // Simulate checking lives
        if (Math.random() < 0.1) { // 10% chance of losing a life
            this.lives -= 1;
            this.heartsDisplay = '❤'.repeat(this.lives);
            console.log(`Lost a heart. Remaining: ${this.heartsDisplay}`);
            return this.lives <= 0;
        }
        return false;
    }

    endGame() {
        if (this.gameLoopRunning) {
            clearInterval(this.intervalId);
            this.gameLoopRunning = false;
        }
        console.log('Game over.');
    }

    enterMenuState() {
        this.menuState = true;
        console.log('Entering menu state.');
    }

    exitMenuState() {
        this.menuState = false;
        console.log('Exiting menu state.');
    }
}

// Example usage
const app = new LegacyApp();
app.exitMenuState(); // Ensure we start in game state
app.startGame(); // Start the game
