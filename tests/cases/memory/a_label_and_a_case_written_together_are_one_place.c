int f(int n) {
    int k;
    k = 3;
    switch (n) {
    case 0:
    again:
    case 1:
        k = k - 1;
        if (k > 0) goto again;
    }
    return k;
}
