void *malloc(unsigned long long size);
void free(void *pointer);

struct Two {
    int *first;
    int *second;
};

struct Mixed {
    int count;
    int *pointer;
};

struct Wide {
    long long count;
    int *pointer;
};

int local(void) {
    int x = 0;
    struct Two two;
    two.second = &x;
    int *t = two.second;
    return t == 0;
}

int heap(void) {
    int x = 0;
    struct Two *two = malloc(sizeof *two);
    if (!two) {
        return 1;
    }
    two->second = &x;
    int *t = two->second;
    free(two);
    return t == 0;
}

int beside_an_int(void) {
    int x = 0;
    struct Mixed m;
    m.count = 1;
    m.pointer = &x;
    int *t = m.pointer;
    return t == 0;
}

int beside_a_long_long(void) {
    int x = 0;
    struct Wide w;
    w.count = 1;
    w.pointer = &x;
    int *t = w.pointer;
    return t == 0;
}

int main(void) {
    return local() + heap() + beside_an_int() + beside_a_long_long();
}
