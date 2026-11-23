void *malloc(unsigned long long size);
void free(void *pointer);

struct Pair {
    int a;
    int b;
};

int local(void) {
    struct Pair x;
    x.a = 0;
    x.b = 1;
    return x.a + x.b - 1;
}

int heap(void) {
    struct Pair *p = malloc(sizeof *p);
    if (!p) {
        return 1;
    }
    p->a = 5;
    p->b = 0;
    int r = p->a + p->b;
    free(p);
    return r - 5;
}

int main(void) {
    return local() + heap();
}
