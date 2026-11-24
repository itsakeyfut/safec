int of_arrays_of_them(int n) {
    int a[n][n];
    return (int)sizeof a;
}

int in_a_static(int n) {
    int a[n];
    static int x = sizeof a;
    return x;
}

int in_a_case(int k, int n) {
    int a[n];
    switch (k) {
    case sizeof a:
        return 1;
    }
    return 0;
}
