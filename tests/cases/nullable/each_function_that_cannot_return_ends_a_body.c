void abort(void);
void exit(int status);
void _Exit(int status);
void quick_exit(int status);

int * _Nonnull a(int *p) {
    if (p) {
        return p;
    }
    abort();
}

int * _Nonnull b(int *p) {
    if (p) {
        return p;
    }
    exit(1);
}

int * _Nonnull c(int *p) {
    if (p) {
        return p;
    }
    _Exit(1);
}

int * _Nonnull d(int *p) {
    if (p) {
        return p;
    }
    quick_exit(1);
}
