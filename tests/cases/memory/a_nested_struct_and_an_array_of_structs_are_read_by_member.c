void *malloc(unsigned long long size);
void free(void *pointer);

struct Inner {
    int *pointer;
};

struct Outer {
    int count;
    struct Inner inner;
};

int nested_never_written(void) {
    struct Outer *o = malloc(sizeof *o);
    if (!o) {
        return 1;
    }
    o->count = 1;
    int *t = o->inner.pointer;
    free(o);
    return t == 0;
}

int element_never_written(void) {
    int x = 0;
    struct Inner many[2];
    many[0].pointer = &x;
    int *t = many[1].pointer;
    return t == 0;
}

int past_the_inner_member(void) {
    struct Outer o;
    o.count = 0;
    int **m = &o.inner.pointer;
    m[1] = 0;
    return o.count;
}

int main(void) {
    return nested_never_written() + element_never_written() + past_the_inner_member();
}
