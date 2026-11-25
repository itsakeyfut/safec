int x = 1;
int z = 1;
int F = 2;
#define z (z+1)
#define F(a) a
#define G F
#define H G *
#define UNUSED(a) 7
#define DECL(d) d
#define SUB(a) a-1
#define register id(4)
#define SQ(x) ((x) * (x))
#define x x+1
#define id(y) y
#define FIRST(a, b) a
#define NONE() 7
#define NEG(v) -v
#define WRAP(x) INNER(x)
#define INNER(x) x
#define DROP(x)
#define K(int) (int + 1)

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
    if (WRAP(WRAP(3)) != 3) fails |= 512;
    DROP(fails |= 1024;)
    if (K(2) != 3) fails |= 2048;
    if (id(z) != 2) fails |= 4096;
    if (H (0) != 0) fails |= 8192;
    if (UNUSED(FIRST(1)) != 7) fails |= 16384;
    DECL(int d = 3;)
    if (d != 3) fails |= 32768;
    if (SUB(5 -) != 6) fails |= 65536;
    if (id(register) != 4) fails |= 131072;
    return fails != 0;
}
