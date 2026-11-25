int x = 1;
#define SQ(x) ((x) * (x))
#define x x+1
#define id(y) y
#define FIRST(a, b) a
#define NONE() 7
#define NEG(v) -v

int main(void) {
    int fails = 0;
    if (SQ(3) != 9) fails |= 1;
    if (SQ(
          2) != 4) fails |= 2;
    if (SQ(SQ(2)) != 16) fails |= 4;
    if (FIRST((0, 2), 3) != 2) fails |= 8;
    int y = 5;
    if (id(y) != 5) fails |= 16;
    if (NONE() != 7) fails |= 32;
    if (NEG(-1) != 1) fails |= 64;
    if (id(x) != 2) fails |= 128;
    int SQ = 1;
    if (SQ != 1) fails |= 256;
    return fails;
}
