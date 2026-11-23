void *malloc(unsigned long long size);
void free(void *pointer);

struct Holder {
    int *pointer;
};

struct Count {
    int a;
    int n;
};

int assigned(void) {
    struct Holder a;
    struct Holder b;
    a.pointer = malloc(sizeof(int));
    if (!a.pointer) {
        return 1;
    }
    *a.pointer = 0;
    b = a;
    free(a.pointer);
    return *b.pointer;
}

static int read_through(struct Holder h) {
    return *h.pointer;
}

int passed(void) {
    struct Holder h;
    h.pointer = malloc(sizeof(int));
    if (!h.pointer) {
        return 1;
    }
    *h.pointer = 0;
    free(h.pointer);
    return read_through(h);
}

static int read_a_member(struct Count c) {
    return c.a + c.n;
}

int counted(void) {
    struct Count c;
    c.a = 1;
    c.n = 2;
    return read_a_member(c) - 3;
}

int main(void) {
    return assigned() + passed() + counted();
}
