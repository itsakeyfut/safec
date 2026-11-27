struct Node {
    struct Node *next;
    int value;
};

int named(void) {
    int x = 4;
    int *ps[3] = {[2] = &x};
    return *ps[2] + (ps[0] == 0);
}

int past(void) {
    int a[4] = {[3] = 1};
    return a[4];
}
