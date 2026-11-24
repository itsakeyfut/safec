struct big {
    long long a;
    long long b;
    long long c;
};

struct big make(long long v);
long long sum(struct big b);

long long through_pointers(void) {
    struct big (*m)(long long) = make;
    long long (*s)(struct big) = sum;
    struct big b = m(1);
    return s(b);
}
