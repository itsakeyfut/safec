struct S;
struct L;
unsigned long long early(void) {
    return sizeof(struct L);
}
struct L {
    int x;
};
int f(void);
int a[];
unsigned long long g(void) {
    return sizeof f + sizeof(void) + sizeof(struct S) + sizeof a;
}
