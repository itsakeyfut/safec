struct S0 {
    int x;
};
struct S1 {
    struct S0 m;
};
struct S2 {
    struct S1 m;
};
struct S3 {
    struct S2 m;
};
struct S4 {
    struct S3 m;
};
struct S5 {
    struct S4 m;
};
struct S6 {
    struct S5 m;
};
struct S7 {
    struct S6 m;
};
struct S8 {
    struct S7 m;
};
struct S9 {
    struct S8 m;
};
struct S10 {
    struct S9 m;
};
struct S11 {
    struct S10 m;
};
struct S12 {
    struct S11 m;
};
struct S13 {
    struct S12 m;
};
struct S14 {
    struct S13 m;
};
struct S15 {
    struct S14 m;
};
struct S16 {
    struct S15 m;
};
struct S17 {
    struct S16 m;
};
struct S18 {
    struct S17 m;
};
struct S19 {
    struct S18 m;
};
struct S20 {
    struct S19 m;
};
struct S21 {
    struct S20 m;
};
struct S22 {
    struct S21 m;
};
struct S23 {
    struct S22 m;
};
struct S24 {
    struct S23 m;
};
struct S25 {
    struct S24 m;
};
struct S26 {
    struct S25 m;
};
struct S27 {
    struct S26 m;
};
struct S28 {
    struct S27 m;
};
struct S29 {
    struct S28 m;
};
struct S30 {
    struct S29 m;
};
struct S31 {
    struct S30 m;
};
struct S32 {
    struct S31 m;
};
struct S33 {
    struct S32 m;
};
struct S34 {
    struct S33 m;
};
struct S35 {
    struct S34 m;
};
struct S36 {
    struct S35 m;
};
struct S37 {
    struct S36 m;
};
struct S38 {
    struct S37 m;
};
struct S39 {
    struct S38 m;
};
struct S40 {
    struct S39 m;
};
int main(void) {
    return sizeof(struct S40) == 4 ? 0 : 1;
}
