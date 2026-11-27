union U {
    const int *c;
    int *p;
};

int main(void) {
    union U u;
    u.c = (const int[]){1, 2};
    int *q = u.p;
    if (q) {
        *q = 3;
    }
    return 0;
}
