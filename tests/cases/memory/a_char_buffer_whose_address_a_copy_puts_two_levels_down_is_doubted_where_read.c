// `cb_` is written at one byte and read as a pointer through `r`, which
// the copy put two levels below `m`. The store spelling, `**m = v_;`, doubts
// it, and the copy now does too (#686).
void *malloc(int n);
void *memcpy(void *d, void *s, int n);
int f(int k) {
    char cb_[8];
    cb_[0] = 0;
    void *cv_ = cb_;
    int **x = cv_;
    int ****m = malloc(8);
    if (!m) return 0;
    int ***n = malloc(8);
    if (!n) return 0;
    *m = n;
    int **v_ = x;
    memcpy(&**m, &v_, 8);
    int **r = **m;
    if (!r) return 0;
    int *p_ = *r;
    return p_ == 0;
}
