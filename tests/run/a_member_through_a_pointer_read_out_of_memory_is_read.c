struct S {
    int a;
    int b;
};

struct Holder {
    struct S *p;
};

struct S *global_pointer;

static int through_two(struct S **pp) {
    (**pp).a = 5;
    (**pp).a += 1;
    (**pp).b++;
    int *address = &(**pp).b;
    return (**pp).a + *address;
}

int main(void) {
    struct S s;
    s.a = 0;
    s.b = 0;
    struct S *p = &s;
    int r = through_two(&p) == 7 ? 0 : 1;
    global_pointer = &s;
    (*global_pointer).a = 2;
    r = r + (s.a == 2 ? 0 : 1);
    struct Holder holder;
    holder.p = &s;
    (*holder.p).b = 3;
    r = r + (s.b == 3 ? 0 : 1);
    return r;
}
