int one(int n) {
    int s = 0;
    for (int i = 0; i < n; i = i + 1) {
        s = s + i;
    }
    return s;
}

int two(int n) {
    for (int i = 0, j = n; i < j; i = i + 1) {
        j = j - 1;
    }
    return 0;
}
