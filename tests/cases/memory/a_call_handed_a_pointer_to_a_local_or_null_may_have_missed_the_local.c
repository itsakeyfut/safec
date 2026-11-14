// A call this check cannot read, handed a pointer that is `&a` on one path
// and null on the other, hands `a`'s address over on one path only, so it
// may not have written `a`: each read of `a` below is `SC0411` (#950). A
// pointer tested not null first is handed `&a` alone, and the call is
// believed to write `a`, as an output parameter is.
void fill(int **p);
void stash(int ***p);
void tick(void);

int null_then_a(int k) {
    int *a;
    int **p = 0;
    if (k) p = &a;
    fill(p);
    return a == 0;
}

int a_then_null(int k) {
    int *a;
    int **p = &a;
    if (k) p = 0;
    fill(p);
    return a == 0;
}

int *returned(int k) {
    int *a;
    int **p = 0;
    if (k) p = &a;
    fill(p);
    return a;
}

int kept_then_called(int k) {
    int *a;
    int **p;
    stash(&p);
    p = 0;
    if (k) p = &a;
    tick();
    return a == 0;
}

int tested_first(int k) {
    int *a;
    int **p = 0;
    if (k) p = &a;
    if (p) {
        fill(p);
        return a == 0;
    }
    return 0;
}
