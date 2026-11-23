struct S;
int f(void);
int a[];
unsigned long long g(void) {
    return sizeof f + sizeof(void) + sizeof(struct S) + sizeof a;
}
