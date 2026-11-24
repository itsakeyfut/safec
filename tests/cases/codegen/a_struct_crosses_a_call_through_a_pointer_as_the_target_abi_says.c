struct big {
    long long a;
    long long b;
    long long c;
};

struct big make(long long v);
long long sum(struct big b);

struct pair {
    long long a;
    long long b;
};

struct pair two(long long v);
long long both(struct pair p);

long long through_pointers(void) {
    struct big (*m)(long long) = make;
    long long (*s)(struct big) = sum;
    struct big b = m(1);
    return s(b);
}

long long in_registers(void) {
    struct pair (*t)(long long) = two;
    long long (*s)(struct pair) = both;
    struct pair p = t(1);
    return s(p);
}

long long an_address_as_an_integer(void) {
    return (long long)two;
}
