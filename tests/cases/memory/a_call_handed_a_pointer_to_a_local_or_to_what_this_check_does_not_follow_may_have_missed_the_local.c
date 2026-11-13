// A call handed a pointer that may be `&a`, or on the other path whatever
// an earlier call filled in, may have written the other place and left
// `a` as it was: `SC0411` where `a` is read, as when the other place is a
// second local. Counted as naming `a` alone, it was believed to write `a`;
// found by review.
void fill(int **p, int *x);
void get(int ***out);
int main(int argc, char **argv) {
    int x = 0;
    int *a;
    int **p;
    get(&p);
    if (argc > 1) {
        p = &a;
    }
    fill(p, &x);
    if (a == 0) return 1;
    return 0;
}
