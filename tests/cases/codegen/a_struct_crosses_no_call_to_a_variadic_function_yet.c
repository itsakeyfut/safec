struct pair {
    long long a;
    long long b;
};

int takes(struct pair p, ...);
struct pair makes(int a, ...);

int fixed_parameter(struct pair p) {
    return takes(p, 1);
}

long long returned(void) {
    struct pair p = makes(1, 2);
    return p.a;
}
