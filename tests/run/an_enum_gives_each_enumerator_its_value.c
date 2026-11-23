enum Colour { RED, GREEN = 5, BLUE, LAST = BLUE * 2 };
typedef enum { NEGATIVE = -2, MINUS_ONE, ZERO, } Sign;

static int score(enum Colour c) {
    switch (c) {
    case RED:
        return 1;
    case GREEN:
        return 2;
    case BLUE:
        return 3;
    case LAST:
        return 4;
    }
    return 0;
}

int main(void) {
    int r = 0;
    enum Colour c = BLUE;
    Sign s = ZERO;
    r = r + (RED == 0 ? 0 : 1);
    r = r + (c == 6 ? 0 : 1);
    r = r + (LAST == 12 ? 0 : 1);
    r = r + (NEGATIVE + MINUS_ONE + s == -3 ? 0 : 1);
    r = r + (score(RED) + score(GREEN) * 10 + score(BLUE) * 100 + score(LAST) * 1000 == 4321 ? 0 : 1);
    r = r + (sizeof(enum Colour) == sizeof(int) ? 0 : 1);
    {
        enum { RED = 7, LENGTH = RED + 1 };
        int a[LENGTH];
        r = r + (sizeof a / sizeof a[0] == 8 ? 0 : 1);
    }
    {
        int RED = 9;
        r = r + (RED == 9 ? 0 : 1);
    }
    return r + RED;
}
