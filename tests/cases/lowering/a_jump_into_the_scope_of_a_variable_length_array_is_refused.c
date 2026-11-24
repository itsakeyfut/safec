int into_a_block(int n) {
    goto inside;
    {
        int a[n];
    inside:
        return (int)sizeof a;
    }
}

int past_a_case(int k, int n) {
    switch (k) {
        int a[n];
    case 1:
        return (int)sizeof a;
    }
    return 0;
}

int past_it_in_the_body(int n) {
    goto after;
    char a[n];
after:
    return (int)sizeof a;
}
