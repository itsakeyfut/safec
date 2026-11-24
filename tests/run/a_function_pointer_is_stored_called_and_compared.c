struct big {
    long long a;
    long long b;
    long long c;
};

struct ops {
    int (*add)(int, int);
    struct big (*make)(long long);
};

int add(int x, int y) {
    return x + y;
}

int twice(int x) {
    return x * 2;
}

struct big make(long long v) {
    struct big b;
    b.a = v;
    b.b = v + 1;
    b.c = v + 2;
    return b;
}

long long sum(struct big b) {
    return b.a + b.b + b.c;
}

int apply(int (*f)(int), int v) {
    return f(v);
}

int (*chosen(int which))(int) {
    return which ? twice : 0;
}

int main(void) {
    int wrong = 0;

    // Stored in a local, called through, by `(*f)()` too, and compared.
    int (*f)(int) = twice;
    wrong += f(3) == 6 ? 0 : 1;
    wrong += (*f)(4) == 8 ? 0 : 1;
    wrong += f == twice ? 0 : 1;
    wrong += f != 0 ? 0 : 1;

    // Held in an array and in a struct member.
    int (*table[2])(int);
    table[0] = 0;
    table[1] = twice;
    wrong += table[1](5) == 10 ? 0 : 1;
    struct ops ops;
    ops.add = add;
    ops.make = make;
    wrong += ops.add(2, 3) == 5 ? 0 : 1;

    // A struct returned and passed by value through a pointer.
    struct big b = ops.make(10);
    long long (*s)(struct big) = sum;
    wrong += s(b) == 33 ? 0 : 1;

    // Passed as a parameter, and returned.
    wrong += apply(twice, 7) == 14 ? 0 : 1;
    wrong += chosen(1)(6) == 12 ? 0 : 1;
    wrong += chosen(0) == 0 ? 0 : 1;

    // A designator called through `*` and `&`.
    wrong += (*twice)(1) + (&twice)(1) == 4 ? 0 : 1;
    return wrong;
}
