#define XS_(...) #__VA_ARGS__
#define XS(...) XS_(__VA_ARGS__)
#define E
#define F(a, ...) f(a, ## __VA_ARGS__)
#define LC(a,...) [a ,##__VA_ARGS__]
#define LS(a, ...) [a , ## __VA_ARGS__ ]
#define Z(x, ...) z(x , ## __VA_ARGS__ ## 1)
#define OP(...) [__VA_OPT__((__VA_ARGS__))]
#define OS(...) [__VA_OPT__( )]
#define Q(x, ...) __VA_OPT__(x)
#define Q2(a, ...) a __VA_OPT__(+1)
#define VI(...) __VA_OPT__(#__VA_ARGS__ a ## b)
#define CAT(x, ...) x ## __VA_ARGS__
#define SO3(...) #__VA_OPT__(a(__VA_ARGS__))
#define ONLY(...) g(0, ## __VA_ARGS__)
#define VC(...) __VA_OPT__(f(0 , ## __VA_ARGS__))
#define PI(x, ...) __VA_OPT__(x ## 2)
#define HI(...) __VA_OPT__(#__VA_ARGS__)
#define EV(...) int __VA_OPT__() ev = 1;

int streq(const char *a, const char *b) {
    while (*a && *a == *b) {
        a++;
        b++;
    }
    return *a == *b;
}

int main(void) {
    long fails = 0;
    EV(1)
    if (!streq(XS(F(F(1), F(2,3))), "f(f(1), F(2,3))")) fails |= 1L << 0;
    if (!streq(XS(LC(1)), "[1]")) fails |= 1L << 1;
    if (!streq(XS(LC(1, 2)), "[1 , 2]")) fails |= 1L << 2;
    if (!streq(XS(LC(1,)), "[1 ,]")) fails |= 1L << 3;
    if (!streq(XS(LS(1)), "[1 ]")) fails |= 1L << 4;
    if (!streq(XS(Z(a, b)), "z(a , b1)")) fails |= 1L << 5;
    if (!streq(XS([ E]), "[ ]")) fails |= 1L << 6;
    if (!streq(XS(F(1, E)), "f(1, )")) fails |= 1L << 7;
    if (!streq(XS(OP(1)), "[(1)]")) fails |= 1L << 8;
    if (!streq(XS(OS(1)), "[ ]")) fails |= 1L << 9;
    if (!streq(XS(Q(5, 1)), "5")) fails |= 1L << 10;
    if (!streq(XS(Q(5)), "")) fails |= 1L << 11;
    if (!streq(XS(Q2(1, z)), "1 +1")) fails |= 1L << 12;
    if (!streq(XS(Q2(1)), "1")) fails |= 1L << 13;
    if (!streq(XS(VI(q r)), "\"q r\" ab")) fails |= 1L << 14;
    if (!streq(XS(CAT(a, b)), "ab")) fails |= 1L << 15;
    if (!streq(XS(CAT(ab)), "ab")) fails |= 1L << 16;
    if (!streq(SO3(1), "a(1)")) fails |= 1L << 17;
    if (!streq(XS(ONLY(1)), "g(0,1)")) fails |= 1L << 18;
    if (!streq(XS(LC(,)), "[ ,]")) fails |= 1L << 19;
    if (!streq(XS(VC(1)), "f(0 ,1)")) fails |= 1L << 20;
    if (!streq(XS(PI(a, 1)), "a2")) fails |= 1L << 21;
    if (!streq(XS(HI(p  q)), "\"p q\"")) fails |= 1L << 22;
    if (ev != 1) fails |= 1L << 30;
    return fails != 0;
}
